"""Offline checks for full-desktop acceptance and read-only preview boundaries."""
import base64
import json
import threading
import unittest
import urllib.error
import urllib.request
from desktop_preview import Preview, decode_frame, handler
import http.server
from desktop_smoke import trace_checks, record_trace
from pathlib import Path
import tempfile
from audit_desktop import audit
from unittest.mock import patch


class DesktopTests(unittest.TestCase):
    def test_native_app_identity_does_not_depend_on_decorated_titles(self):
        trace = [{'tool': 'cua_get_window_state', 'status': 'completed', 'error': None,
                  'image_returned': True, 'output': json.dumps({'observation': {
                      'app_name': 'Thunar', 'window_title': 'Archive'}})}]
        checks = trace_checks(trace)
        self.assertTrue(checks['file_manager_observed'])
        self.assertTrue(checks['archive_navigated'])
        self.assertFalse(checks['document_saved_and_reopened'])
        self.assertFalse(checks['file_drag_dispatched'])

    def test_same_document_window_is_not_a_close_and_reopen(self):
        trace = [{'tool': 'cua_get_window_state', 'status': 'completed', 'error': None,
                  'image_returned': True, 'output': json.dumps({'observation': {
                      'pid': 123, 'window_id': 9, 'window_title': 'Reliability report — LibreOffice Writer'}})}] * 2
        self.assertFalse(trace_checks(trace)['document_saved_and_reopened'])
        trace[1] = {**trace[1], 'output': trace[1]['output'].replace('123', '124')}
        # Two different windows alone cannot prove the file was reopened.
        self.assertFalse(trace_checks(trace)['document_saved_and_reopened'])

    def test_start_center_then_file_manager_open_proves_reopen_with_same_xid(self):
        def observation(state):
            return {'tool': 'cua_get_window_state', 'status': 'completed', 'error': None,
                    'image_returned': True, 'output': json.dumps({'observation': state})}
        writer = {'pid': 123, 'window_id': 9,
                  'window_title': 'Reliability report.odt - LibreOffice Writer'}
        start_center = {**writer, 'window_title': 'LibreOffice'}
        documents = {'pid': 100, 'window_id': 5, 'app_name': 'Thunar', 'window_title': 'Documents'}
        activate = {'tool': 'cua_press_key', 'status': 'completed', 'error': None,
                    'arguments': {'pid': 100, 'window_id': 5, 'key': 'ENTER'}, 'output': '{}'}
        prefix = [observation(writer), observation(start_center), observation(documents)]
        self.assertFalse(trace_checks(prefix + [observation(writer)])['document_saved_and_reopened'])
        self.assertTrue(trace_checks(prefix + [activate, observation(writer)])['document_saved_and_reopened'])
        self.assertFalse(trace_checks(prefix + [{**activate, 'status': 'failed'}, observation(writer)])['document_saved_and_reopened'])

    def test_tool_updates_are_one_execution_in_checkpoint_evidence(self):
        item = {'id': 'one', 'type': 'toolExecution', 'toolName': 'cua_click',
                'input': {}, 'status': 'inProgress'}
        with tempfile.TemporaryDirectory() as directory:
            trace = record_trace([item, {**item, 'status': 'failed', 'error': 'refused'}], Path(directory))
        self.assertEqual(len(trace), 1)
        self.assertEqual(trace[0]['status'], 'failed')

    def test_an_audit_cannot_convert_failed_files_or_missing_gui_steps_to_pass(self):
        report = {'sandbox': 'owned', 'model': 'test', 'trace': [], 'passed': True,
                  'turn_status': 'completed', 'extension_registered': True,
                  'model_received_images': True, 'app_server_stopped': True,
                  'independent_grader': {'passed': False}}
        result = audit(report)
        self.assertFalse(result['passed'])
        self.assertFalse(result['gui_input_replayed'])
        self.assertTrue(result['original_acceptance'])

    def test_preview_rejects_failed_truncated_or_invalid_images(self):
        jpeg = base64.b64encode(b'\xff\xd8example\xff\xd9').decode()
        process = {'exitCode': 0, 'stdout': json.dumps({'jpeg': jpeg, 'width': 1920, 'height': 1080})}
        self.assertEqual(decode_frame(process)[1], {'width': 1920, 'height': 1080})
        for malformed in [{'exitCode': 1, 'stdout': process['stdout']},
                          {'exitCode': 0, 'stdout': '{'},
                          {'exitCode': 0, 'stdout': json.dumps({'jpeg': 'AA==', 'width': 10, 'height': 10})},
                          {'exitCode': 0, 'stdout': json.dumps({'jpeg': jpeg, 'width': 0, 'height': 10})}]:
            with self.subTest(process=malformed), self.assertRaises(ValueError):
                decode_frame(malformed)

    def test_expired_preview_stops_cloud_reads_and_preserves_the_last_frame(self):
        preview = Preview('owned-test', 'example', duration=0)
        preview.frame = b'last-real-frame'
        with patch.object(preview.transport, 'run_script') as cloud:
            preview.capture()
            cloud.assert_not_called()
        frame, status = preview.snapshot()
        self.assertEqual(frame, b'last-real-frame')
        self.assertEqual(status['state'], 'preview ended')

    def test_viewer_exposes_only_the_cached_read_only_frame(self):
        preview = Preview('owned-test', 'example')
        preview.frame = b'cached-screen'
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler(preview))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        url = f'http://127.0.0.1:{server.server_port}'
        try:
            with urllib.request.urlopen(url + '/frame') as response:
                self.assertEqual(response.read(), b'cached-screen')
                self.assertEqual(response.headers['Cache-Control'], 'no-store')
            for path, headers, code in [('/input', {}, 404), ('/frame', {'Host': 'external.example'}, 403)]:
                with self.subTest(path=path), self.assertRaises(urllib.error.HTTPError) as error:
                    urllib.request.urlopen(urllib.request.Request(url + path, headers=headers))
                self.assertEqual(error.exception.code, code)
                error.exception.close()
        finally:
            server.shutdown(); server.server_close(); thread.join()


if __name__ == '__main__':
    unittest.main()
