#!/usr/bin/env python3
"""Recheck retained desktop evidence without replaying any GUI input."""
import argparse
import json
from pathlib import Path
from desktop_smoke import trace_checks


def audit(report):
    checks = trace_checks(report['trace'])
    cleanup = report.get('cleanup') or {}
    checks.update(public_turn_completed=report.get('turn_status') == 'completed',
                  extension_registered=report.get('extension_registered') is True,
                  model_received_images=report.get('model_received_images') is True,
                  independent_files_passed=report.get('independent_grader', {}).get('passed') is True,
                  app_server_stopped=report.get('app_server_stopped') is True,
                  cleanup_or_explicit_retention=(report.get('sandbox_retained_for_evaluation') is True
                      or (cleanup.get('delete_accepted') is True and cleanup.get('status') == 'TERMINATED')))
    return {'passed': all(checks.values()), 'checks': checks,
            'sandbox': report['sandbox'], 'model': report['model'],
            'original_acceptance': report.get('passed'),
            'original_trace_checks': report.get('trace_checks'),
            'method': 'Read-only audit of the original public tool trace and independent file grader',
            'reopen_rule': 'Saved report closes; File Manager opens from Documents; fresh Writer observation follows. Native XID may be reused.',
            'gui_input_replayed': False}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    result = audit(json.loads(args.report.read_text()))
    text = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(text)
    print(text, end='')
    if not result['passed']:
        raise SystemExit(1)
