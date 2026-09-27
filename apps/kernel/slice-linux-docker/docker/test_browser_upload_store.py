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

    def test_preparing_requires_whole_container_retirement_proof(self):
        owner = {"boot": "boot-a", "namespace": "pid:[123]", "uid": os.getuid()}
        lifetime = {"instance": uuid.uuid4().hex, "supervisor": owner}
        entry = {"phase": "preparing", "owner": owner, "lifetime": lifetime}
        directory = store.lifecycle.directory()
        store.lifecycle.write_json(directory / f"{lifetime['instance']}.retired.json", lifetime)
        self.assertFalse(store.container_retired(entry))
        proof = {"version": 1, "authority": "docker-stopped-container", "engineId": "engine-1234",
                 "containerId": "a" * 64, "startedAt": "2026-09-27T19:00:00Z", "finishedAt": "2026-09-27T19:01:00Z", "lifetime": lifetime}
        filename = directory / f"{lifetime['instance']}.container-retired.json"
        store.lifecycle.write_json(filename, proof)
        self.assertTrue(store.container_retired(entry))
        self.assertFalse(store.container_retired({**entry, "owner": {**owner, "namespace": "other"}}))
        store.lifecycle.write_json(filename, {**proof, "lifetime": {**lifetime, "instance": uuid.uuid4().hex}})
        self.assertFalse(store.container_retired(entry))

    def test_unknown_preparing_controller_reclaimed_only_with_container_proof(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "uploads"
            root.mkdir(mode=0o700)
            entry = {"id": str(uuid.uuid4()), "bytes": 8, "count": 1, "phase": "preparing", "owner": {}, "lifetime": None}
            (root / entry["id"]).mkdir()
            store.lifecycle.write_json(root / "ledger.json", [entry])
            with patch.object(store, "process_state", return_value="unknown"), patch.object(store, "container_retired", return_value=False):
                store.transact(root, {"action": "reap"})
            self.assertTrue((root / entry["id"]).exists())
            with patch.object(store, "process_state", return_value="unknown"), patch.object(store, "container_retired", return_value=True):
                store.transact(root, {"action": "reap"})
            self.assertFalse((root / entry["id"]).exists())
            self.assertEqual(json.loads((root / "ledger.json").read_text()), [])

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
