import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class ViewerRuntimeTest(unittest.TestCase):
    def test_staged_runtime_can_start_and_retire_browser_lifetime(self):
        source = Path(__file__).parent
        spec = importlib.util.spec_from_file_location("viewer", source / "validate-slice-viewer.py")
        viewer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(viewer)
        with tempfile.TemporaryDirectory(prefix="chariox-viewer-runtime-test-") as scratch:
            root = Path(scratch)
            viewer.prepare_runtime(source / "docker", root)
            environment = {**os.environ, "TMPDIR": scratch,
                           "CHARIOX_BROWSER_LIFECYCLE_ROOT": str(root / "lifecycle")}
            helper = ["python3", str(root / "browser-lifecycle.py")]
            profile = str(root / "profile")
            browser = root / "chromium-fixture.py"
            browser.write_text("import time; time.sleep(30)\n")
            try:
                started = subprocess.run([*helper, "start", profile, str(root / "browser.log"),
                                          "python3", str(browser), profile],
                                         env=environment, capture_output=True, text=True, check=True, timeout=10)
                record = json.loads(started.stdout)
                browser_launch = viewer.owned_signals.record(record["browser"]["pid"])
                viewer.crash_chromium(browser_launch)
            finally:
                stopped = subprocess.run([*helper, "stop", profile], env=environment,
                                         capture_output=True, text=True, timeout=10)
            self.assertEqual(stopped.returncode, 0, stopped.stderr)
            self.assertTrue(list((root / "lifecycle").glob("*.retired.json")))


if __name__ == "__main__":
    unittest.main()
