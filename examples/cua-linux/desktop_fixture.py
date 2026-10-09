"""Full XFCE desktop provisioning and independent read-only file grading."""
import json
from fixture import provision, BlaxelTransport, launch_input_fixture


def provision_desktop(transport):
    provision(transport, launch_calculator=False)
    script = '''import hashlib,json,os,pathlib,pwd,subprocess,time,uuid
subprocess.run(['apt-get','install','-y','-qq','--no-install-recommends',
 'thunar','xfce4-appfinder','mousepad','libreoffice-writer','libreoffice-gtk3',
 'fonts-dejavu'],check=True,stdout=subprocess.DEVNULL,
 env={**os.environ,'DEBIAN_FRONTEND':'noninteractive'})
prefix=json.loads(pathlib.Path('/opt/roder-cua/session.json').read_text())
subprocess.run(prefix+['xrandr','--output','VNC-0','--mode','1920x1080'],check=True,capture_output=True)
account=pwd.getpwnam('cua')
root=pathlib.Path('/home/cua/Desktop/Roder Desktop Test')
if root.exists():raise RuntimeError('Full desktop acceptance requires a fresh sandbox; fixture already exists')
for directory in [root,root/'Archive',root/'Documents']:
 directory.mkdir(parents=True,exist_ok=True)
 os.chown(directory,account.pw_uid,account.pw_gid)
marker='RODER-DESKTOP-'+uuid.uuid4().hex[:12]
seed='Native file drag seed '+marker+'\\n'
source=root/'Source note.txt';source.write_text(seed)
os.chown(source,account.pw_uid,account.pw_gid)
subprocess.run(prefix+['xfconf-query','-c','xfce4-keyboard-shortcuts','-p',
 '/commands/custom/<Super>d','-n','-t','string','-s','xfce4-appfinder --disable-server'],
 check=True,capture_output=True)
packages=subprocess.check_output(['dpkg-query','-W','-f=${Package}=${Version}\\n',
 'xfce4-session','xfwm4','thunar','xfce4-appfinder','libreoffice-writer'],text=True).splitlines()
profile={'backend':'XFCE/X11','driver_version':'0.34.0','marker':marker,'screen_width':1920,'screen_height':1080,
 'root':str(root),'source':str(source),'destination':str(root/'Archive/Source note.txt'),
 'document':str(root/'Documents/Reliability report.odt'),
 'seed_sha256':hashlib.sha256(seed.encode()).hexdigest(),'packages':packages,
 'initial_setup':'Only directories and seed file; document does not exist'}
pathlib.Path('/opt/roder-cua/desktop-profile.json').write_text(json.dumps(profile,indent=2))
log=open('/tmp/roder-cua-appfinder.log','w')
subprocess.Popen(prefix+['xfce4-appfinder','--disable-server'],stdin=subprocess.DEVNULL,
 stdout=log,stderr=log,start_new_session=True)
print(json.dumps(profile))
'''
    result = transport.run_script(script, timeout=600)
    if result.get('exitCode') != 0:
        raise RuntimeError('Full desktop provisioning failed: ' + result.get('stderr', '')[-2000:])
    return json.loads(result['stdout'])


def grade_desktop(transport):
    script = '''import hashlib,json,pathlib,zipfile,xml.etree.ElementTree as ET
profile=json.loads(pathlib.Path('/opt/roder-cua/desktop-profile.json').read_text())
source=pathlib.Path(profile['source']);destination=pathlib.Path(profile['destination'])
document=pathlib.Path(profile['document'])
checks={'source_moved':not source.exists(),'destination_exists':destination.is_file(),
 'file_bytes_preserved':destination.is_file() and hashlib.sha256(destination.read_bytes()).hexdigest()==profile['seed_sha256'],
 'document_exists':document.is_file()}
text='';mime=None
if document.is_file():
 try:
  with zipfile.ZipFile(document) as archive:
   mime=archive.read('mimetype').decode()
   tree=ET.fromstring(archive.read('content.xml'))
   text='\\n'.join(''.join(node.itertext()) for node in tree.iter() if node.tag.endswith('}p') or node.tag.endswith('}h'))
 except (OSError,ValueError,KeyError,zipfile.BadZipFile,ET.ParseError):pass
checks.update(document_is_odt=mime=='application/vnd.oasis.opendocument.text',
 document_contains_marker=profile['marker'] in text,
 document_contains_report='Roder desktop reliability report' in text)
print(json.dumps({'passed':all(checks.values()),'checks':checks,'document_text':text,
 'document_sha256':hashlib.sha256(document.read_bytes()).hexdigest() if document.is_file() else None,
 'destination_sha256':hashlib.sha256(destination.read_bytes()).hexdigest() if destination.is_file() else None,
 'profile':profile,'grader':'read-only filesystem bytes and ODT content.xml'}))
'''
    result = transport.run_script(script)
    if result.get('exitCode') != 0:
        raise RuntimeError('Desktop read-only grader failed')
    return json.loads(result['stdout'])


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sandbox', required=True)
    parser.add_argument('--workspace', required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--grade', action='store_true')
    mode.add_argument('--input-fixture', action='store_true', help='Also start the native input oracle after provisioning')
    args = parser.parse_args()
    transport = BlaxelTransport(args.sandbox, args.workspace)
    profile = (grade_desktop if args.grade else provision_desktop)(transport)
    if args.input_fixture:
        launch_input_fixture(transport)
    print(json.dumps(profile, indent=2))
