import base64
import io
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from main import CuaExtension, serve
from transport import BlaxelTransport, image_result


class RecordingTransport:
    def __init__(self):
        self.calls = []

    def call(self, name, arguments):
        self.calls.append((name, arguments))
        return {"windows": []}, False


class BoundaryTests(unittest.TestCase):
    def test_input_requires_trusted_launcher_opt_in(self):
        transport = RecordingTransport()
        extension = CuaExtension(transport)
        with self.assertRaisesRegex(ValueError, "input disabled"):
            extension.call_tool({"toolName": "cua_click", "threadId": "t1", "arguments": {}})
        self.assertEqual(transport.calls, [])

    def test_thread_cannot_retarget_the_child_sandbox(self):
        transport = RecordingTransport()
        extension = CuaExtension(transport)
        extension.call_tool({"toolName": "cua_list_windows", "threadId": "t1", "arguments": {}})
        with self.assertRaisesRegex(ValueError, "another thread"):
            extension.call_tool({"toolName": "cua_list_windows", "threadId": "t2", "arguments": {}})
        self.assertEqual(len(transport.calls), 1)

    def test_arguments_cannot_override_routing_or_inject_a_session(self):
        transport = RecordingTransport()
        extension = CuaExtension(transport, allow_input=True)
        for invalid in ({"session": "other"}, {"sandbox": "other"}, {"pid": True, "window_id": 3, "x": 4, "y": 5}):
            with self.assertRaises(ValueError):
                extension.call_tool({"toolName": "cua_click", "threadId": "t1", "arguments": invalid})
        self.assertEqual(transport.calls, [])

    def test_actual_capture_fixture_is_image_content_without_base64_text(self):
        fixture = Path(__file__).resolve().parents[1] / "evidence/2026-10-08/final.png"
        encoded = base64.b64encode(fixture.read_bytes()).decode()
        result = image_result({"screenshot_png_b64": encoded, "screenshot_mime_type": "image/png", "value": "42"}, False)
        self.assertNotIn(encoded, result["content"])
        self.assertNotIn("screenshot_png_b64", result["data"]["observation"])
        self.assertEqual(base64.b64decode(result["data"]["__view_image"]["image_url"].split(",", 1)[1]), fixture.read_bytes())
        self.assertTrue(result["data"]["untrusted"])

    def test_invalid_image_and_bad_sandbox_fail_closed(self):
        with self.assertRaises(ValueError):
            image_result({"screenshot_png_b64": base64.b64encode(b"not a PNG").decode()}, False)
        with self.assertRaises(ValueError):
            BlaxelTransport("x; touch /tmp/no", "workspace")

    def test_malformed_notification_does_not_crash_protocol(self):
        output = io.StringIO()
        serve(CuaExtension(RecordingTransport()), io.StringIO('broken\n[]\n{"id":1,"method":"extension/initialize"}\n'), output)
        self.assertEqual(json.loads(output.getvalue())["result"]["extensionId"], "cua-linux-spike")


if __name__ == "__main__":
    unittest.main()
