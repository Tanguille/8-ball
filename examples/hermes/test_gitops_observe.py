import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest


SCRIPT = Path(__file__).with_name("gitops_observe.py")


class ObserverTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="8ball-observe-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.snapshot = {"context": "uses kopiur", "items": [{"id": "a", "title": "backup change"}, {"id": "b", "title": "routine bump"}]}
        self.input = self.root / "input.json"
        self.input.write_text(json.dumps(self.snapshot))
        self.observations = self.root / "run.jsonl"
        self.requests = []

    def server(self, status=200, redirect=None, truncated=False):
        requests = self.requests

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                requests.append((self.path, dict(self.headers), json.loads(self.rfile.read(int(self.headers["Content-Length"])))))
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                if redirect:
                    self.send_header("Location", redirect)
                if truncated:
                    self.send_header("Content-Length", "99999")
                self.end_headers()
                self.wfile.write(json.dumps({"kept": [], "unsure": [], "rejected": [{"index": 0}, {"index": 1}], "prefiltered": [], "dropped": 2}).encode())

            def do_GET(self):
                requests.append((self.path, dict(self.headers), None))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"{}")

            def log_message(self, *_args):
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return f"http://127.0.0.1:{server.server_port}"

    def run_observer(self, url):
        result = subprocess.run([sys.executable, str(SCRIPT), "--input", str(self.input), "--observations", str(self.observations)],
            env={**os.environ, "EIGHTBALL_URL": url, "EIGHTBALL_API_KEY": "test-key"}, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        output = json.loads(result.stdout)
        self.assertTrue(output["wakeAgent"])
        self.assertEqual(output["context"]["snapshot"], self.snapshot)
        return output

    def test_model_rejections_never_remove_candidates(self):
        self.run_observer(self.server())
        self.assertEqual(len(self.requests), 1)
        path, headers, body = self.requests[0]
        self.assertEqual(path, "/v1/triage")
        self.assertEqual(headers["Authorization"], "Bearer test-key")
        self.assertEqual(body["context"], self.snapshot["context"])
        self.assertEqual(body["items"], self.snapshot["items"])
        self.assertNotIn("top_k", body)
        record = json.loads(self.observations.read_text())
        self.assertEqual(record["mode"], "observe")
        self.assertEqual(record["snapshot"], self.snapshot)
        self.assertEqual(record["result"]["dropped"], 2)
        self.assertIsNone(record["error"])
        self.assertEqual(self.observations.stat().st_mode & 0o777, 0o600)

    def test_backend_failure_keeps_every_candidate_and_records_error(self):
        self.run_observer(self.server(502))
        self.assertIn("502", json.loads(self.observations.read_text())["error"])

    def test_existing_observation_is_never_overwritten(self):
        self.observations.write_text("previous run\n")
        output = self.run_observer(self.server())
        self.assertEqual(self.observations.read_text(), "previous run\n")
        self.assertIn("observation_error", output["context"])

    def test_truncated_response_keeps_work_and_records_error(self):
        self.run_observer(self.server(truncated=True))
        self.assertIsInstance(json.loads(self.observations.read_text())["error"], str)

    def test_redirect_is_not_followed_with_credentials(self):
        destination = self.server()
        self.run_observer(self.server(302, redirect=destination + "/unexpected"))
        self.assertEqual(len(self.requests), 1)
        self.assertIn("302", json.loads(self.observations.read_text())["error"])

    def test_missing_snapshot_wakes_agent_with_error_instead_of_empty_success(self):
        self.input.unlink()
        result = subprocess.run([sys.executable, str(SCRIPT), "--input", str(self.input), "--observations", str(self.observations)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        output = json.loads(result.stdout)
        self.assertTrue(output["wakeAgent"])
        self.assertIn("input_error", output["context"])


if __name__ == "__main__":
    unittest.main()
