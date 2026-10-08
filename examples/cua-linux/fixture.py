"""Provision and independently grade the owned native Linux calculator."""
import json
from pathlib import Path
import sys

EXAMPLE = Path(__file__).resolve().parent
sys.path.insert(0, str(EXAMPLE.parent / 'non-rust-extensions' / 'cua-linux'))
from transport import BlaxelTransport

def provision(transport):
    installer = (EXAMPLE / 'install.py').read_text()
    launcher = (EXAMPLE / 'cua_call.py').read_text()
    script = f'''import pathlib,subprocess,json
path=pathlib.Path('/tmp/roder-cua-install');path.mkdir(exist_ok=True)
(path/'install.py').write_text({installer!r})
(path/'cua_call.py').write_text({launcher!r})
subprocess.run(['apt-get','update','-qq'],check=True,stdout=subprocess.DEVNULL)
subprocess.run(['apt-get','install','-y','-qq','galculator','libxi6','at-spi2-core','python3-gi','gir1.2-atspi-2.0'],check=True,stdout=subprocess.DEVNULL)
subprocess.run(['python3',str(path/'install.py')],check=True)
prefix=json.loads(pathlib.Path('/opt/roder-cua/session.json').read_text())
log=open('/tmp/roder-calculator.log','w')
subprocess.Popen(prefix+['galculator'],stdin=subprocess.DEVNULL,stdout=log,stderr=log,start_new_session=True)
'''
    result = transport.run_script(script, timeout=300)
    if result.get('exitCode') != 0:
        raise RuntimeError('Desktop provisioning failed: ' + result.get('stderr','')[-900:])

def grade(transport):
    reader = """import gi,json
gi.require_version('Atspi','2.0')
from gi.repository import Atspi
values=[]
def visit(node,depth=0):
 if depth>15:return
 try:
  if node.get_role_name()=='text':
   text=node.get_text_iface()
   values.append(text.get_text(0,text.get_character_count()) if text else node.get_name())
  for i in range(node.get_child_count()):visit(node.get_child_at_index(i),depth+1)
 except Exception:pass
desktop=Atspi.get_desktop(0)
for i in range(desktop.get_child_count()):
 app=desktop.get_child_at_index(i)
 if app.get_name().lower()=='galculator':visit(app)
print(json.dumps({'display_values':values,'passed':'42' in values}))
"""
    script = ("import json,pathlib,subprocess\n"
              "prefix=json.loads(pathlib.Path('/opt/roder-cua/session.json').read_text())\n"
              f"p=subprocess.run(prefix+['/usr/bin/python3','-c',{reader!r}],capture_output=True,text=True,timeout=30)\n"
              "print(p.stdout);raise SystemExit(p.returncode)\n")
    result = transport.run_script(script)
    if result.get('exitCode') != 0:
        raise RuntimeError('Independent app grader failed')
    return json.loads(result['stdout'])


def provision_input(transport):
    provision(transport)
    source = (EXAMPLE / 'input_fixture.py').read_text()
    script = f"""import pathlib,subprocess,json,time
subprocess.run(['apt-get','install','-y','-qq','gir1.2-gtk-3.0'],check=True,stdout=subprocess.DEVNULL)
pathlib.Path('/tmp/roder-cua-input-fixture.py').write_text({source!r})
prefix=json.loads(pathlib.Path('/opt/roder-cua/session.json').read_text())
log=open('/tmp/roder-cua-input-fixture.log','w')
subprocess.Popen(prefix+['/usr/bin/python3','/tmp/roder-cua-input-fixture.py'],stdout=log,stderr=log,start_new_session=True)
for _ in range(80):
 if pathlib.Path('/tmp/roder-cua-input-oracle.json').exists():break
 time.sleep(.1)
else:raise RuntimeError('Input fixture failed: '+pathlib.Path('/tmp/roder-cua-input-fixture.log').read_text()[-1200:])
print('Native input fixture ready')
"""
    result = transport.run_script(script, timeout=120)
    if result.get('exitCode') != 0:
        raise RuntimeError('Input provisioning failed: ' + result.get('stderr','')[-1200:])
