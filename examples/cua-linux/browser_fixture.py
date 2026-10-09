"""Chrome on the full X11 desktop, plus an independent loopback website oracle."""
import json
from pathlib import Path
from fixture import BlaxelTransport


def provision_browser(transport):
    source = Path(__file__).with_name('browser_site.py').read_text()
    script = f'''import hashlib,json,os,pathlib,pwd,subprocess,time,urllib.request,uuid
root=pathlib.Path('/opt/roder-cua')
if (root/'browser-profile.json').exists():raise RuntimeError('Browser qualification requires a fresh profile')
url='https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb'
data=urllib.request.urlopen(url,timeout=120).read()
package=pathlib.Path('/tmp/roder-chrome.deb');package.write_bytes(data)
subprocess.run(['apt-get','install','-y','-qq',str(package)],check=True,stdout=subprocess.DEVNULL,
 env={{**os.environ,'DEBIAN_FRONTEND':'noninteractive'}})
profile={{'marker':'RODER-BROWSER-'+uuid.uuid4().hex[:12],'url':'http://127.0.0.1:8765',
 'session_cookie':uuid.uuid4().hex,'expected_title':'Roder browser reliability report',
 'expected_notes':'Browser control works: café λ 日本語',
 'chrome_version':subprocess.check_output(['google-chrome','--version'],text=True).strip(),
 'chrome_package_sha256':hashlib.sha256(data).hexdigest(),
 'profile_path':'/home/cua/.config/google-chrome','driver_version':'0.34.0',
 'profile_authorization':'Explicit driver existing-profile grant on this disposable test desktop'}}
(root/'browser-profile.json').write_text(json.dumps(profile));(root/'browser-profile.json').chmod(0o600)
(root/'browser-site.py').write_text({source!r})
log=open('/tmp/roder-browser-site.log','w')
subprocess.Popen(['/usr/bin/python3',str(root/'browser-site.py')],stdout=log,stderr=log,start_new_session=True)
prefix=json.loads((root/'session.json').read_text())
# This trusted deployment file grants only attachment to this owned test desktop.
# A model/tool argument cannot write or replace the daemon's launch-time grant.
(root/'driver-grants.json').write_text(json.dumps(['existing-profile']));(root/'driver-grants.json').chmod(0o600)
account=pwd.getpwnam('cua')
preferences=pathlib.Path(profile['profile_path'])/'Default/Preferences';preferences.parent.mkdir(parents=True,exist_ok=True)
preferences.write_text(json.dumps({{'browser':{{'has_seen_welcome_page':True,'check_default_browser':False}},
 'distribution':{{'skip_first_run_ui':True}},'signin':{{'allowed':False}},
 'default_apps':'noinstall','privacy_sandbox':{{'m1':{{'consent_decision_made':True}}}}}}))
for path in [preferences,preferences.parent,preferences.parent.parent]:os.chown(path,account.pw_uid,account.pw_gid)
chrome_log=open('/tmp/roder-chrome.log','w')
process=subprocess.Popen(prefix+['env','ACCESSIBILITY_ENABLED=1','google-chrome','--no-sandbox','--disable-dev-shm-usage',
 '--no-first-run','--no-default-browser-check','--disable-features=PrivacySandboxSettings4',
 '--force-renderer-accessibility',profile['url']],stdout=chrome_log,stderr=chrome_log,start_new_session=True)
profile['launch_supervisor_pid']=process.pid
(root/'browser-profile.json').write_text(json.dumps(profile));(root/'browser-profile.json').chmod(0o600)
# The legacy image can report accessibility disabled during Chrome startup.
# Force its native ATK bridge as well as renderer accessibility, then prove it.
probe="import gi,json;gi.require_version('Atspi','2.0');from gi.repository import Atspi;d=Atspi.get_desktop(0);print(json.dumps([dict(name=d.get_child_at_index(i).get_name(),children=d.get_child_at_index(i).get_child_count()) for i in range(d.get_child_count())]))"
for _ in range(40):
 result=subprocess.run(prefix+['/usr/bin/python3','-c',probe],capture_output=True,text=True,timeout=15)
 apps=json.loads(result.stdout) if result.returncode==0 else []
 if any('chrome' in a['name'].lower() and a['children']>0 for a in apps):break
 time.sleep(.25)
else:raise RuntimeError('Chrome did not register its native accessibility bridge')
profile['native_accessibility_ready']=True
(root/'browser-profile.json').write_text(json.dumps(profile));(root/'browser-profile.json').chmod(0o600)
public={{k:v for k,v in profile.items() if k!='session_cookie'}}
print(json.dumps(public,ensure_ascii=False))
'''
    result = transport.run_script(script, timeout=600)
    if result.get('exitCode') != 0:
        raise RuntimeError('Browser provisioning failed: ' + result.get('stderr','')[-1800:])
    return json.loads(result['stdout'])


def grade_browser(transport):
    script = '''import json,pathlib
profile=json.loads(pathlib.Path('/opt/roder-cua/browser-profile.json').read_text())
oracle=json.loads(pathlib.Path('/tmp/roder-cua-browser-oracle.json').read_text())
submission=oracle['submission'] or {}
checks={'signed_in_via_site':any(e['path']=='/login' and e['method']=='POST' for e in oracle['events']),
 'authenticated_submission':any(e['path']=='/submit' and e['signed_in'] for e in oracle['events']),
 'title_matches':submission.get('title')==profile['expected_title'],
 'unicode_and_marker_match':submission.get('notes')==profile['expected_notes']+' '+profile['marker'],
 'authenticated_receipt_read':any(e['path']=='/receipt' and e['signed_in'] for e in oracle['events']),
 'profile_retained':pathlib.Path(profile['profile_path'],'Default/Cookies').is_file() or pathlib.Path(profile['profile_path'],'Default/Network/Cookies').is_file()}
print(json.dumps({'passed':all(checks.values()),'checks':checks,'oracle':oracle,
 'profile':{k:v for k,v in profile.items() if k!='session_cookie'},
 'grader':'Read-only independent website HTTP event log and submitted form state'},ensure_ascii=False))
'''
    result=transport.run_script(script)
    if result.get('exitCode') != 0:raise RuntimeError('Browser grader failed')
    return json.loads(result['stdout'])

if __name__ == '__main__':
    import argparse
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--sandbox',required=True);p.add_argument('--workspace',required=True);p.add_argument('--grade',action='store_true')
    a=p.parse_args();print(json.dumps((grade_browser if a.grade else provision_browser)(BlaxelTransport(a.sandbox,a.workspace)),indent=2,ensure_ascii=False))
