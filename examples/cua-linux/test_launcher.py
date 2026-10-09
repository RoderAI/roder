"""Offline boundary checks for the installed runner-side launcher."""
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import cua_call


class LauncherTests(unittest.TestCase):
    def test_text_only_tool_reply_retains_the_drivers_refusal(self):
        self.assertEqual(cua_call.observation_reply('element_not_editable: no input dispatched\n'),
                         {'summary': 'element_not_editable: no input dispatched', 'effect': 'unverifiable'})
        self.assertEqual(cua_call.observation_reply('{"code":"refused"}'), {'code': 'refused'})

    def test_missing_malformed_or_unbounded_reply_is_not_a_success(self):
        for raw in ['', '  ', '{"code":', '[{}]', 'null', 'x' * 8193]:
            with self.subTest(raw=raw[:50]), self.assertRaises(ValueError):
                cua_call.observation_reply(raw)

    def test_text_refusal_from_worker_remains_an_error_without_replay(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'session.json').write_text('[]')
            destination = root / 'result.json'
            replies = [subprocess.CompletedProcess([], 0, 'cua-driver 0.34.0\n', ''),
                       subprocess.CompletedProcess([], 1, 'Window target is stale; refresh list_windows.\n', '')]
            with patch.object(cua_call, 'ROOT', root), \
                    patch.object(cua_call, 'validate', return_value=({'session': 'roder-test'}, destination)), \
                    patch.object(cua_call.os, 'getppid', return_value=12345), \
                    patch.object(cua_call, 'daemon_ready', return_value=True), \
                    patch.object(cua_call.subprocess, 'run', side_effect=replies) as command:
                cua_call.worker('get_window_state', '{}', 'ignored', 12345)
            wire = json.loads(destination.read_text())
            self.assertTrue(wire['is_error'])
            self.assertIn('Window target is stale', wire['observation']['summary'])
            self.assertEqual(command.call_count, 2)  # Version probe and one dispatch.

    def test_stale_socket_is_recovered_without_replaying_an_action(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory)/'driver.sock')
            server = socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
            server.bind(path)
            server.close()
            with patch.object(cua_call,'SOCKET',path):
                self.assertFalse(cua_call.daemon_ready())
                self.assertFalse(Path(path).exists())

    def test_live_socket_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory)/'driver.sock')
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen(1)
                with patch.object(cua_call,'SOCKET',path):
                    self.assertTrue(cua_call.daemon_ready())
                    self.assertTrue(Path(path).exists())

    def test_non_socket_cannot_be_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'driver.sock'
            path.write_text('keep')
            with patch.object(cua_call,'SOCKET',str(path)):
                with self.assertRaises(Exception):
                    cua_call.daemon_ready()
            self.assertEqual(path.read_text(),'keep')

    def test_model_cannot_select_tool_or_result_location(self):
        for tool,path in [('shell','/tmp/roder-cua-results/'+'a'*32+'.json'),
                          ('click','/tmp/foreign.json'),
                          ('click','/tmp/roder-cua-results/../../foreign.json')]:
            with self.subTest(tool=tool,path=path), patch.object(cua_call.sys,'argv',
                    ['cua-call',tool,json.dumps({'session':'roder-test'}),path]):
                with self.assertRaises(ValueError):
                    cua_call.main()

    def test_abandoned_worker_does_not_dispatch_late_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            with patch.object(cua_call,'ROOT',root), patch.object(cua_call,'validate',
                    return_value=({'session':'roder-test'},root/'result.json')), \
                    patch.object(cua_call.os,'getppid',return_value=1), \
                    patch.object(cua_call.subprocess,'run') as command:
                cua_call.worker('click','{}','ignored',12345)
                command.assert_not_called()
                self.assertFalse((root/'result.json').exists())


if __name__=='__main__':
    unittest.main()
