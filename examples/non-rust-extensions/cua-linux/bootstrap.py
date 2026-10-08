#!/usr/bin/env python3
"""Provision an existing *disposable* Blaxel XFCE sandbox for this spike."""

import argparse
import json

from transport import BlaxelTransport

DRIVER_VERSION = "0.34.0"
DRIVER_SHA256 = "629ac96eff829d4dfd5cf221f3f2165c2d813aed91e5efb7b20777a741cd70a7"

REMOTE_SETUP = r'''
import hashlib,json,os,pathlib,subprocess,tarfile,time,urllib.request
if pathlib.Path('/tmp/roder-cua.sock').exists():
 raise RuntimeError('Driver already bootstrapped; refuse to replace a live desktop. Use a fresh sandbox.')
dest=pathlib.Path('/tmp/roder-cua-driver');dest.mkdir(exist_ok=True)
binary=dest/'cua-driver'
asset='cua-driver-rs-0.34.0-linux-x86_64-binary.tar.gz'
url='https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.34.0/'+asset
data=urllib.request.urlopen(url,timeout=60).read()
assert hashlib.sha256(data).hexdigest()=='629ac96eff829d4dfd5cf221f3f2165c2d813aed91e5efb7b20777a741cd70a7'
archive=dest/asset;archive.write_bytes(data)
with tarfile.open(archive) as handle:handle.extractall(dest,filter='data')
subprocess.run(['apt-get','update','-qq'],check=True,stdout=subprocess.DEVNULL)
subprocess.run(['apt-get','install','-y','-qq','galculator','libxi6','at-spi2-core','python3-gi','gir1.2-atspi-2.0'],check=True,stdout=subprocess.DEVNULL)
pid=next(p.name for p in pathlib.Path('/proc').iterdir() if p.name.isdigit() and (p/'comm').exists() and (p/'comm').read_text().strip()=='xfce4-session')
session={}
for item in pathlib.Path('/proc',pid,'environ').read_bytes().split(b'\0'):
 if b'=' in item:
  key,value=item.split(b'=',1);session[key.decode()]=value.decode(errors='replace')
selected={k:session[k] for k in ['DISPLAY','DBUS_SESSION_BUS_ADDRESS','XDG_RUNTIME_DIR','XAUTHORITY'] if k in session}
selected.update(HOME='/home/cua',CUA_DRIVER_RS_TELEMETRY_ENABLED='0')
prefix=['runuser','-u','cua','--','env']+[k+'='+v for k,v in selected.items()]
pathlib.Path('/tmp/roder-cua-session.json').write_text(json.dumps(prefix))
for label,args in [('driver',[str(binary),'serve','--socket','/tmp/roder-cua.sock','--no-overlay']),('calculator',['galculator'])]:
 log=open('/tmp/roder-cua-'+label+'.log','w')
 subprocess.Popen(prefix+args,stdout=log,stderr=log,start_new_session=True)
for attempt in range(60):
 if pathlib.Path('/tmp/roder-cua.sock').exists():break
 time.sleep(.25)
if not pathlib.Path('/tmp/roder-cua.sock').exists():raise RuntimeError('Cua daemon did not become ready')
print(json.dumps({'driver_version':'0.34.0','display':selected.get('DISPLAY'),'backend':'X11/XFCE','calculator':'galculator','sha256':'629ac96eff829d4dfd5cf221f3f2165c2d813aed91e5efb7b20777a741cd70a7'}))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sandbox", required=True, help="Owned disposable blaxel/cua-xfce sandbox")
    parser.add_argument("--workspace", required=True)
    args = parser.parse_args()
    process = BlaxelTransport(args.sandbox, args.workspace).run_script(REMOTE_SETUP, timeout=300)
    if process.get("exitCode") != 0:
        raise SystemExit("Bootstrap failed: " + process.get("stderr", "")[-1200:])
    print(process["stdout"])


if __name__ == "__main__":
    main()
