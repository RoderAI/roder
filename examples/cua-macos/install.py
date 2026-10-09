#!/usr/bin/env python3
"""Install the qualified signed CuaDriver release on Apple Silicon macOS.
Never overwrites another release or changes macOS privacy grants.
"""
import argparse
import hashlib
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import urllib.request

VERSION='0.34.0'
SHA256='329bcc140c4840a5877e2cfc9f756351eb4a70c2c2d6acf4954751918122c60a'
URL=f'https://github.com/trycua/cua/releases/download/cua-driver-rs-v{VERSION}/cua-driver-rs-{VERSION}-darwin-arm64.tar.gz'


def verify(app):
    subprocess.run(['/usr/bin/codesign','--verify','--deep','--strict',str(app)],check=True)
    subprocess.run(['/usr/sbin/spctl','--assess','--type','execute',str(app)],check=True)
    version=subprocess.check_output([str(app/'Contents/MacOS/cua-driver'),'--version'],text=True).split()[-1]
    if version!=VERSION:raise RuntimeError(f'Expected Cua {VERSION}; found {version}. Choose an empty app directory for the pinned release.')


def install(app_dir):
    if platform.system()!='Darwin' or platform.machine()!='arm64':
        raise RuntimeError('This qualified installer requires Apple Silicon macOS')
    app=app_dir/'CuaDriver.app'
    if app.exists():
        verify(app)
        return app
    with tempfile.TemporaryDirectory(prefix='roder-cua-install-') as directory:
        temp=Path(directory); archive=temp/'driver.tar.gz'; digest=hashlib.sha256()
        with urllib.request.urlopen(URL,timeout=60) as response,archive.open('wb') as output:
            size=0
            while block:=response.read(1024*1024):
                size+=len(block)
                if size>150*1024*1024:raise RuntimeError('Release exceeds download bound')
                digest.update(block);output.write(block)
        if digest.hexdigest()!=SHA256:raise RuntimeError('Cua release checksum mismatch')
        with tarfile.open(archive) as handle:handle.extractall(temp,filter='data')
        staged=temp/f'cua-driver-rs-{VERSION}-darwin-arm64/CuaDriver.app'
        verify(staged)
        app_dir.mkdir(parents=True,exist_ok=True)
        if app.exists():raise RuntimeError('Destination appeared during installation; refusing to replace it')
        subprocess.run(['/usr/bin/ditto',str(staged),str(app)],check=True)
        verify(app)
    return app


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app-dir',type=Path,default=Path('/Applications'))
    app=install(parser.parse_args().app_dir.resolve())
    print(f'Installed and verified {app}; version {VERSION}.')
    print(f'Grant privacy access with: "{app}/Contents/MacOS/cua-driver" permissions grant')
    print(f'Launch the app-owned daemon with: open -g "{app}" --args serve')
