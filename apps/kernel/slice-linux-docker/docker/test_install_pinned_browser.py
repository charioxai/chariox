import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-pinned-browser.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class BrowserInstallationTests(unittest.TestCase):
    def fixture(self, root, filename="chrome-linux64/chrome"):
        data = io.BytesIO()
        with zipfile.ZipFile(data, "w") as bundle:
            entry = zipfile.ZipInfo(filename)
            entry.external_attr = 0o100755 << 16
            bundle.writestr(entry, b"#!/bin/sh\nexit 0\n")
        payload = data.getvalue()
        lock = {"schema": "chariox.kernel-browser-chromium.v1", "version": "155.test",
                "linux64": {"url": "https://example.invalid/browser.zip", "executable": "chrome-linux64/chrome",
                            "sha256": hashlib.sha256(payload).hexdigest(), "size": len(payload)}}
        path = root / "lock.json"
        path.write_text(json.dumps(lock))
        return path, payload

    def test_installs_the_verified_executable_and_startup_link(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock, payload = self.fixture(root)
            with patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(payload)):
                installer.install(lock, root / "installed")
            binary = root / "installed/chromium/155.test/chrome-linux64/chrome"
            self.assertEqual((root / "installed/chrome").resolve(), binary)
            self.assertEqual(binary.stat().st_mode & 0o777, 0o755)

    def test_rejects_modified_archives_before_installing(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock, payload = self.fixture(root)
            with patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(payload + b"changed")):
                with self.assertRaisesRegex(ValueError, "approved pin"):
                    installer.install(lock, root / "installed")
            self.assertFalse((root / "installed").exists())

    def test_rejects_archive_paths_outside_the_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock, payload = self.fixture(root, "../../../escaped")
            with patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(payload)):
                with self.assertRaisesRegex(ValueError, "unsafe"):
                    installer.install(lock, root / "installed")
            self.assertFalse((root / "escaped").exists())


if __name__ == "__main__":
    unittest.main()
