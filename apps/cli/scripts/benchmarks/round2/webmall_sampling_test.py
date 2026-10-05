"""MP-08 / MP-10 / MP-11 H6: unchanged official evaluator, real external CDP.

Synthetic fixture only: no benchmark tasks, provider, gold or real accounts.
Run with WEBMALL_OFFICIAL_SOURCE at the frozen BrowserGym webmall/src root.
"""
import contextlib
import hashlib
import http.server
import importlib
import io
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import types
import unittest

from playwright.sync_api import sync_playwright
from webmall_sampling import fresh_existing_pages

SOURCE = Path(os.environ["WEBMALL_OFFICIAL_SOURCE"]).resolve()
# Load the unchanged modules without importing unrelated BrowserGym runtimes.
package = types.ModuleType("webmall_fixture")
package.__path__ = [str(SOURCE / "browsergym/webmall")]
sys.modules[package.__name__] = package
evaluator = importlib.import_module("webmall_fixture.evaluator")
checkpoints = importlib.import_module("webmall_fixture.checkpoints")


class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        text = {"/front/right": "https://shop.example/product/alpha https://shop.example/product/beta",
                "/front/partial": "https://shop.example/product/alpha",
                "/front/wrong": "https://shop.example/product/other"}.get(self.path, "")
        body = ("<title>MP-08 MP-10 synthetic shop</title><div id=submittedResult>" + text + "</div>").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class ExternalNavigation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = "http://127.0.0.1:" + str(cls.server.server_port)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            cls.port = sock.getsockname()[1]
        cls.pw = sync_playwright().start()
        # Playwright owns its child lifecycle; no custom signal/group helper.
        cls.browser = cls.pw.chromium.launch(headless=True, args=[f"--remote-debugging-port={cls.port}"])
        cls.context = cls.browser.new_context()
        cls.pages = [cls.context.new_page(), cls.context.new_page()]

    @classmethod
    def tearDownClass(cls):
        cls.browser.close()
        cls.pw.stop()
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def evaluate_external_submission(self, case):
        for i, page in enumerate(self.pages):
            page.goto(self.base + f"/shop/{i}")
        target = self.base + "/front/" + case
        os.environ["FRONTEND_URL"] = target
        # The independent connection delivers a navigation while the grader's
        # synchronous Playwright loop is idle. No grader-owned goto/input.
        code = """
import sys
from playwright.sync_api import sync_playwright
with sync_playwright() as p:
 b=p.chromium.connect_over_cdp(sys.argv[1])
 pages=[page for ctx in b.contexts for page in ctx.pages]
 page=next(page for page in pages if page.url.endswith('/shop/1'))
 page.goto(sys.argv[2]); assert page.locator('#submittedResult').text_content()
"""
        child = subprocess.run([sys.executable, "-c", code, f"http://127.0.0.1:{self.port}", target],
                               capture_output=True, timeout=30)
        self.assertEqual(child.returncode, 0, "MP-10 external fixture navigation failed")
        self.assertTrue(self.pages[1].url.endswith("/shop/1"), "MP-10 expected cached pre-pump URL")
        cps = [checkpoints.Checkpoint("answer1", "https://shop.example/product/alpha", "string", weight=.5),
               checkpoints.Checkpoint("answer2", "https://shop.example/product/beta", "string", weight=.5)]
        grader = evaluator.StringEvaluator()
        # Prove the historical seam before applying the passive observation.
        self.assertEqual(grader.eval("", self.pages[1], cps), (0, []))
        self.assertFalse(grader.done)
        total, wrong = 0, []
        with contextlib.redirect_stdout(io.StringIO()):
            for page in fresh_existing_pages(self.context):
                score, errors = grader.eval("", page, cps)
                total += score
                wrong.extend(errors)
                if grader.done:
                    break  # exact first terminal validation; no re-grading
        self.assertTrue(grader.done, "MP-08/MP-10 H6 cached URL starves official evaluator")
        self.assertEqual(self.pages[1].url, target)
        self.assertEqual(self.pages[1].locator("#submittedResult").text_content(),
                         {"right": "https://shop.example/product/alpha https://shop.example/product/beta",
                          "partial": "https://shop.example/product/alpha",
                          "wrong": "https://shop.example/product/other"}[case])
        return total, wrong, [cp.flag for cp in cps]

    def test_right_set_multiple_tabs(self):
        self.assertEqual(self.evaluate_external_submission("right"), (1, [], [True, True]))

    def test_partial_set_retains_official_partial_score(self):
        self.assertEqual(self.evaluate_external_submission("partial"), (.5, [], [True, False]))

    def test_wrong_set_retains_official_zero(self):
        self.assertEqual(self.evaluate_external_submission("wrong"),
                         (0, ["https://shop.example/product/other"], [False, False]))


class TransientObservation(unittest.TestCase):
    def test_sample_omits_unobserved_page_and_retains_error_class(self):
        class ClosingPage:
            url = "https://fixture.example/stale"

            def title(self):
                raise RuntimeError("MP-10 synthetic closing tab")

        page = ClosingPage()
        errors = []
        context = types.SimpleNamespace(pages=[page])
        self.assertEqual(fresh_existing_pages(context, strict=False, on_error=errors.append), [])
        self.assertEqual(errors, [{"phase": "passive_observation", "pageOrdinal": 0, "errorClass": "RuntimeError"}])
        with self.assertRaisesRegex(RuntimeError, "closing tab"):
            fresh_existing_pages(context)


if __name__ == "__main__":
    print("MP-08/MP-10 H6 official evaluator sha256=" + hashlib.sha256(Path(evaluator.__file__).read_bytes()).hexdigest())
    unittest.main()
