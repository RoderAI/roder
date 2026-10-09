#!/bin/bash
# Owns the build/bundle/launch path for the disposable AppKit fixture.
set -euo pipefail
fixture_root="$(cd "$(dirname "$0")/.." && pwd)"
fixture_output="${1:?usage: build_and_run.sh OUTPUT_DIRECTORY [--build-only]}"
mkdir -p "$fixture_output"
fixture_output="$(cd "$fixture_output" && pwd)"
fixture_app="$fixture_output/RoderCuaInput.app"
mkdir -p "$fixture_app/Contents/MacOS"
xcrun swiftc "$fixture_root/InputFixture.swift" -o "$fixture_app/Contents/MacOS/RoderCuaInput"
xcrun swiftc "$fixture_root/calculator_grader.swift" -o "$fixture_output/calculator_grader"
/usr/bin/python3 - "$fixture_app" <<'PY'
import plistlib,sys
from pathlib import Path
app=Path(sys.argv[1])
with (app/'Contents/Info.plist').open('wb') as handle:
    plistlib.dump({'CFBundlePackageType':'APPL','CFBundleExecutable':'RoderCuaInput',
        'CFBundleIdentifier':'ai.roder.cua-input-fixture','CFBundleName':'Roder Cua Input',
        'LSMinimumSystemVersion':'14.0','NSPrincipalClass':'NSApplication'},handle)
PY
/usr/bin/codesign --force --sign - "$fixture_app"
if [[ "${2:-}" != --build-only ]]; then
    # Only stop the exact prior fixture PID recorded in this output directory.
    /usr/bin/python3 - "$fixture_output/state.json" "$fixture_app" <<'PY'
import json,os,subprocess,sys
from pathlib import Path
state=Path(sys.argv[1])
if state.exists():
    pid=json.loads(state.read_text())['pid']
    probe=subprocess.run(['/bin/ps','-p',str(pid),'-o','command='],capture_output=True,text=True)
    expected=str((Path(sys.argv[2])/'Contents/MacOS/RoderCuaInput').resolve())
    if probe.stdout.strip().startswith(expected): os.kill(pid,15)
    state.unlink()
PY
    /usr/bin/open -n "$fixture_app" --args --state "$fixture_output/state.json"
fi

if [[ "${2:-}" != --build-only ]]; then
    /usr/bin/python3 - "$fixture_output/state.json" <<'PYWAIT'
import json,sys,time
from pathlib import Path
state=Path(sys.argv[1]); deadline=time.monotonic()+10
while time.monotonic()<deadline:
    try:
        if json.loads(state.read_text()).get('pid'): break
    except (OSError,ValueError): pass
    time.sleep(0.05)
else: raise RuntimeError('Fixture did not become ready')
PYWAIT
fi
