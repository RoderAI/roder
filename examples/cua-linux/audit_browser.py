#!/usr/bin/env python3
"""Recheck a recorded public-runtime browser trace; never replay GUI input."""
import argparse
import json
from pathlib import Path
from browser_smoke import trace_checks


def audit(report):
    checks=trace_checks(report.get('trace',[]))
    retained=report.get('sandbox_retained_for_evaluation') is True
    cleanup=report.get('cleanup') or {}
    passed=(report.get('turn_status')=='completed' and report.get('extension_registered')
        and report.get('model_received_images') and report.get('app_server_stopped')
        and report.get('independent_grader',{}).get('passed') and all(checks.values())
        and (retained or (cleanup.get('delete_accepted') and cleanup.get('status')=='TERMINATED')))
    return {'passed':bool(passed),'original_acceptance':report.get('passed'),
            'trace_checks':checks,'sandbox':report.get('sandbox'),'gui_input_replayed':False,
            'grader':'Current ordered trace checks against preserved original report'}

if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('report',type=Path);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();result=audit(json.loads(args.report.read_text()));args.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
    raise SystemExit(0 if result['passed'] else 1)
