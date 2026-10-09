#!/usr/bin/env python3
"""Install pinned Cua Driver and launcher inside a Linux graphical sandbox.

Run as root inside the runner, after its XFCE session is ready. This does not
create or own a sandbox. The image must provide X11, D-Bus, AT-SPI and libXi.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import urllib.request

VERSION = '0.34.0'
SHA256 = '629ac96eff829d4dfd5cf221f3f2165c2d813aed91e5efb7b20777a741cd70a7'
ROOT = Path('/opt/roder-cua')


def main():
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        raise RuntimeError('this installer qualifies Linux x86_64 only')
    if os.geteuid() != 0:
        raise RuntimeError('install as root in the owned sandbox')
    ROOT.mkdir(parents=True, exist_ok=True)
    if Path('/tmp/roder-cua.sock').exists():
        raise RuntimeError('stop the existing driver before installation')
    asset = f'cua-driver-rs-{VERSION}-linux-x86_64-binary.tar.gz'
    url = f'https://github.com/trycua/cua/releases/download/cua-driver-rs-v{VERSION}/{asset}'
    data = urllib.request.urlopen(url, timeout=60).read()
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise RuntimeError('driver checksum mismatch')
    with tempfile.TemporaryFile() as archive:
        archive.write(data)
        archive.seek(0)
        with tarfile.open(fileobj=archive) as handle:
            member = next(member for member in handle if member.name == 'cua-driver')
            if not member.isfile():
                raise RuntimeError('driver archive contains no regular binary')
            (ROOT / 'cua-driver').write_bytes(handle.extractfile(member).read())
    (ROOT / 'cua-driver').chmod(0o755)
    # Attach to the actual graphical session, not root's unrelated D-Bus.
    pid = next(entry.name for entry in Path('/proc').iterdir()
               if entry.name.isdigit() and (entry / 'comm').exists()
               and (entry / 'comm').read_text().strip() == 'xfce4-session')
    environ = dict(item.split(b'=', 1) for item in Path('/proc', pid, 'environ').read_bytes().split(b'\0')
                   if b'=' in item)
    selected = {key: environ[key.encode()].decode() for key in
                ['DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'XDG_RUNTIME_DIR', 'XAUTHORITY']
                if key.encode() in environ}
    selected.update(HOME='/home/cua', CUA_DRIVER_RS_TELEMETRY_ENABLED='0')
    if 'DISPLAY' not in selected or 'DBUS_SESSION_BUS_ADDRESS' not in selected:
        raise RuntimeError('graphical/accessibility session is not ready')
    prefix = ['runuser', '-u', 'cua', '--', 'env'] + [f'{key}={value}' for key, value in selected.items()]
    (ROOT / 'session.json').write_text(json.dumps(prefix))
    (ROOT / 'session.json').chmod(0o600)
    (ROOT / 'bin').mkdir(exist_ok=True)
    (ROOT / 'bin' / 'cua-call').write_bytes(Path(__file__).with_name('cua_call.py').read_bytes())
    (ROOT / 'bin' / 'cua-call').chmod(0o755)
    subprocess.run([str(ROOT / 'cua-driver'), '--version'], check=True)
    print(json.dumps({'driver_version': VERSION, 'sha256': SHA256, 'backend': 'XFCE/X11',
                      'program': str(ROOT / 'bin' / 'cua-call')}))


if __name__ == '__main__':
    main()
