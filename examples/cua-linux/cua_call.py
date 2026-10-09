#!/usr/bin/env python3
"""Runner-side Cua 0.34.0 launcher, installed at /opt/roder-cua/bin/cua-call."""
import fcntl
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import sys
import time

ROOT = Path('/opt/roder-cua')
SOCKET = '/tmp/roder-cua.sock'
VERSION = '0.34.0'
TOOLS = {'list_apps', 'list_windows', 'get_window_state', 'get_desktop_state',
         'click', 'drag', 'scroll', 'press_key', 'type_text', 'set_value', 'move_cursor',
         'bring_to_front', 'set_window_frame', 'start_session', 'end_session',
         'get_browser_state', 'browser_prepare', 'browser_navigate', 'browser_click', 'browser_type'}


def daemon_ready():
    """Probe without sending an action, and remove only a refused stale socket."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
        probe.settimeout(1)
        try:
            probe.connect(SOCKET)
            return True
        except FileNotFoundError:
            return False
        except ConnectionRefusedError:
            if stat.S_ISSOCK(Path(SOCKET).lstat().st_mode):
                Path(SOCKET).unlink()
                return False
            raise RuntimeError('driver socket path is not a socket')


def validate(tool, raw_arguments, raw_path):
    if tool not in TOOLS:
        raise ValueError('unsupported Cua invocation')
    path = Path(raw_path)
    if (path.parent != Path('/tmp/roder-cua-results') or path.suffix != '.json'
            or len(path.stem) != 32 or any(c not in '0123456789abcdef' for c in path.stem)):
        raise ValueError('invalid result path')
    arguments = json.loads(raw_arguments)
    if not isinstance(arguments, dict) or not arguments.get('session', '').startswith('roder-'):
        raise ValueError('host-owned session is required')
    return arguments, path


def environment():
    env = os.environ.copy()
    env.pop('RODER_BLAXEL_COMMAND_TAG', None)
    return env


def observation_reply(raw):
    """The pinned CLI emits text when a tool has no structuredContent."""
    try:
        observation = json.loads(raw)
    except ValueError:
        text = raw.strip()
        if not text or len(text) > 8192 or text.startswith(('{', '[')):
            raise ValueError('driver returned no valid observation') from None
        observation = {'summary': text, 'effect': 'unverifiable'}
    if not isinstance(observation, dict):
        raise ValueError('invalid driver observation')
    return observation


def worker(tool, raw_arguments, raw_path, parent):
    arguments, result_path = validate(tool, raw_arguments, raw_path)
    result_path.parent.mkdir(mode=0o700, exist_ok=True)
    # The worker owns this fence independently of its cancellable caller. A
    # disconnected driver call can still be finishing an atomic drag. Preserve
    # the fence until its response, then discard an abandoned result. Never
    # replay input or expose separate press/release operations.
    with (ROOT / 'call.lock').open('a') as lock:
        deadline = time.monotonic()+10
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic()>=deadline:
                    raise RuntimeError('desktop remains busy; no input dispatched')
                time.sleep(.05)
        # A cancelled caller waiting for the fence must not dispatch late.
        if os.getppid()!=parent:
            return
        prefix = json.loads((ROOT / 'session.json').read_text())
        binary = str(ROOT / 'cua-driver')
        version = subprocess.run([binary, '--version'], check=True, capture_output=True,
                                 text=True, timeout=5).stdout.strip().split()[-1]
        if version != VERSION:
            raise RuntimeError('unsupported driver version')
        if not daemon_ready():
            with (ROOT / 'driver.log').open('a') as log:
                grant_path = ROOT / 'driver-grants.json'
                grants = json.loads(grant_path.read_text()) if grant_path.exists() else []
                if grants not in ([], ['existing-profile']):
                    raise RuntimeError('unsupported driver launch grant')
                grant_args = ['--grant', 'existing-profile'] if grants else []
                subprocess.Popen(prefix + [binary, 'serve', '--socket', SOCKET, '--no-overlay'] + grant_args,
                                 stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                 start_new_session=True, env=environment())
            for _ in range(80):
                if daemon_ready():
                    break
                time.sleep(.05)
            else:
                raise RuntimeError('driver did not start')
        if os.getppid()!=parent:
            return
        result = subprocess.run(prefix + [binary, 'call', tool, json.dumps(arguments),
                                         '--socket', SOCKET], capture_output=True, text=True,
                                timeout=40)
        observation = observation_reply(result.stdout)
        wire = json.dumps({'driver_version': VERSION, 'is_error': result.returncode != 0,
                           'observation': observation})
        if len(wire) > 12 * 1024 * 1024:
            raise RuntimeError('response exceeds limit')
        if os.getppid()==parent:
            temporary = result_path.with_suffix('.tmp')
            temporary.write_text(wire)
            temporary.chmod(0o600)
            temporary.replace(result_path)
            if os.getppid()!=parent:
                result_path.unlink(missing_ok=True)


def main():
    if len(sys.argv)!=4:
        raise ValueError('unsupported Cua invocation')
    _, result_path = validate(*sys.argv[1:])
    # A separately owned, bounded worker finishes already-dispatched atomic
    # input under the lock even when runner cancellation kills this supervisor.
    # Its lifetime is <= 10s lock wait + 5s version + 4s start + 40s call.
    with (ROOT / 'driver.log').open('a') as log:
        process = subprocess.Popen([sys.executable, __file__, '--worker', *sys.argv[1:], str(os.getpid())],
            stdin=subprocess.DEVNULL,stdout=log,stderr=log,start_new_session=True,env=environment())
        if process.wait(timeout=60)!=0 or not result_path.is_file():
            raise RuntimeError('driver worker failed')
    print(json.dumps({'driver_version': VERSION, 'result_file': str(result_path)}))


if __name__ == '__main__':
    try:
        if len(sys.argv)==6 and sys.argv[1]=='--worker':
            worker(*sys.argv[2:5],int(sys.argv[5]))
        else:
            main()
    except Exception:
        print('Cua launcher failed; inspect /opt/roder-cua/driver.log inside the owned runner',
              file=sys.stderr)
        raise SystemExit(1)
