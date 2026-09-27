import importlib.util
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch
import uuid


SPEC = importlib.util.spec_from_file_location("upload_store", Path(__file__).with_name("browser-upload-store.py"))
store = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(store)


class UploadOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.environment = patch.dict(os.environ, {"CHARIOX_BROWSER_LIFECYCLE_ROOT": str(Path(self.temporary.name).resolve() / "lifetimes")})
        self.environment.start()
        self.addCleanup(self.temporary.cleanup)
        self.addCleanup(self.environment.stop)

    def test_short_lock_contention_waits_for_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "uploads"
            root.mkdir(mode=0o700)
            descriptor = os.open(root / "quota.lock", os.O_CREAT | os.O_RDWR, 0o600)
            try:
                store.fcntl.flock(descriptor, store.fcntl.LOCK_EX)
                release = threading.Timer(0.05, store.fcntl.flock, args=(descriptor, store.fcntl.LOCK_UN))
                release.start()
                try:
                    store.transact(root, {"action": "reap"})
                finally:
                    release.join()
                self.assertEqual(json.loads((root / "ledger.json").read_text()), [])
            finally:
                os.close(descriptor)

    def test_lock_wait_has_finite_deadline(self):
        with patch.object(store.fcntl, "flock", side_effect=BlockingIOError), patch.object(store.time, "monotonic", side_effect=[0, 0, 2]), patch.object(store.time, "sleep"):
            with self.assertRaises(BlockingIOError):
                store.lock_quota(123)

    def test_unbound_browser_cannot_reserve_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "uploads"
            request = {"action": "reserve", "entry": {"id": str(uuid.uuid4()), "bytes": 8, "count": 1},
                       "maximumBytes": 8, "maximumFiles": 1}
            with patch.object(store, "browser_owner", return_value=None), patch.object(store.lifecycle, "identity", return_value={}):
                with self.assertRaisesRegex(RuntimeError, "owned browser lifetime"):
                    store.transact(root, request)
            self.assertFalse((root / "ledger.json").exists())
            self.assertEqual([path.name for path in root.iterdir()], ["quota.lock"])

    def test_legacy_unbound_reservation_cannot_be_exposed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "uploads"
            root.mkdir(mode=0o700)
            entry = {"id": str(uuid.uuid4()), "bytes": 8, "count": 1, "phase": "preparing", "owner": {}, "lifetime": None}
            (root / "ledger.json").write_text(json.dumps([entry]))
            os.chmod(root / "ledger.json", 0o600)
            with patch.object(store, "process_state", return_value="alive"), patch.object(store.lifecycle, "identity", return_value={}):
                with self.assertRaisesRegex(RuntimeError, "owned browser lifetime"):
                    store.transact(root, {"action": "expose", "id": entry["id"]})
            self.assertEqual(json.loads((root / "ledger.json").read_text()), [entry])


if __name__ == "__main__":
    unittest.main()
