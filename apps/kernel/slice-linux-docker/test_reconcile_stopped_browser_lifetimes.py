import importlib.util
import io
import base64
import json
from pathlib import Path
import tarfile
import unittest

spec = importlib.util.spec_from_file_location("reconciliation", Path(__file__).with_name("reconcile-stopped-browser-lifetimes.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
IDENTITY = "a" * 64
LABELS = {"io.chariox.slice.id": "slice-1", "io.chariox.slice.owner-kernel-id": "kernel-1", "io.chariox.slice.owner-machine-id": "machine-1"}
INSTANCE = "b" * 32
PROCESS = {"pid": 22, "start": "123", "group": 22, "session": 22, "uid": 1001,
           "boot": "12345678-1234-1234-1234-123456789012", "namespace": "pid:[123]"}
RECORD = {"version": 1, "instance": INSTANCE, "profile": "/home/slice/profile", "supervisor": PROCESS, "browser": PROCESS}


def archive(entries=None):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as target:
        for name, value, kind, uid in entries or [(INSTANCE + ".json", RECORD, tarfile.REGTYPE, 1001)]:
            raw = json.dumps(value).encode()
            item = tarfile.TarInfo("chariox-browser-lifecycle-1001/" + name)
            item.size, item.mode, item.uid, item.gid, item.type = len(raw), 0o600, uid, uid, kind
            target.addfile(item, io.BytesIO(raw))
    return output.getvalue()


class Docker:
    def __init__(self, state="exited", data=None, change=False):
        self.state, self.data, self.change, self.reads, self.writes = state, data or archive(), change, 0, []

    def __call__(self, *args, data=None):
        if args[0] == "info":
            return b"engine-1\n"
        if args[:2] == ("system", "dial-stdio"):
            return b"HTTP/1.1 404 Not Found\r\n\r\n"
        if args[0] == "inspect":
            self.reads += 1
            return json.dumps([{"Id": IDENTITY, "Config": {"Labels": LABELS}, "HostConfig": {"PidMode": ""}, "State": {"Status": self.state, "Running": self.state == "running",
                "Paused": self.state == "paused", "Restarting": self.state == "restarting", "Pid": 0,
                "StartedAt": str(self.reads) if self.change else "2026-09-27T00:00:00Z", "FinishedAt": "2026-09-27T01:00:00Z"}}]).encode()
        if args[0] == "cp" and args[-1] == "-":
            return self.data
        if args[:3] == ("cp", "-a", "-"):
            self.writes.append((args, data))
            return b""
        raise AssertionError(args)


class ReconciliationTests(unittest.TestCase):
    def test_interrupted_atomic_write_is_not_lifetime_authority(self):
        docker = Docker(data=archive([(INSTANCE + ".json", RECORD, tarfile.REGTYPE, 1001),
            ("record-abcd1234", {"incomplete": True}, tarfile.REGTYPE, 1001)]))
        self.assertEqual(module.reconcile(IDENTITY, LABELS, docker)["retired"], 1)
        for kind, uid in [(tarfile.SYMTYPE, 1001), (tarfile.REGTYPE, 0)]:
            docker = Docker(data=archive([("record-abcd1234", {}, kind, uid)]))
            with self.assertRaises(RuntimeError):
                module.reconcile(IDENTITY, LABELS, docker)
            self.assertEqual(docker.writes, [])

    def test_exact_stopped_generation_emits_only_owned_missing_receipt(self):
        docker = Docker()
        result = module.reconcile(IDENTITY, LABELS, docker)
        self.assertEqual(result["retired"], 1)
        with tarfile.open(fileobj=io.BytesIO(docker.writes[0][1])) as output:
            item = output.getmembers()[0]
            self.assertEqual(item.name, INSTANCE + ".retired.json")
            self.assertEqual((item.uid, item.gid, item.mode), (1001, 1001, 0o600))
            self.assertEqual(json.load(output.extractfile(item)), RECORD)

    def test_live_or_changed_generation_never_writes(self):
        for docker in [Docker(state=s) for s in ("running", "paused", "restarting")] + [Docker(change=True)]:
            with self.assertRaises(RuntimeError):
                module.reconcile(IDENTITY, LABELS, docker)
            self.assertEqual(docker.writes, [])

    def test_created_container_is_not_retirement_proof(self):
        docker = Docker(state="created")
        self.assertEqual(module.reconcile(IDENTITY, LABELS, docker)["reason"], "never-started-container")
        self.assertEqual(docker.writes, [])

    def test_archive_traversal_symlink_owner_and_schema_rejected(self):
        for entry in [("../../other", RECORD, tarfile.REGTYPE, 1001),
                      (INSTANCE + ".json", RECORD, tarfile.SYMTYPE, 1001),
                      (INSTANCE + ".json", RECORD, tarfile.REGTYPE, 0),
                      (INSTANCE + ".json", {**RECORD, "supervisor": None}, tarfile.REGTYPE, 1001)]:
            docker = Docker(data=archive([entry]))
            with self.assertRaises(RuntimeError):
                module.reconcile(IDENTITY, LABELS, docker)
            self.assertEqual(docker.writes, [])

    def test_different_existing_receipt_is_never_overwritten(self):
        docker = Docker(data=archive([(INSTANCE + ".json", RECORD, tarfile.REGTYPE, 1001),
            (INSTANCE + ".retired.json", {**RECORD, "profile": "/other"}, tarfile.REGTYPE, 1001)]))
        with self.assertRaisesRegex(RuntimeError, "different existing"):
            module.reconcile(IDENTITY, LABELS, docker)
        self.assertEqual(docker.writes, [])

    def test_host_or_shared_pid_mode_never_establishes_retirement(self):
        for mode in ("host", "container:" + IDENTITY):
            docker = Docker()
            def request(*args, **kwargs):
                value = docker(*args, **kwargs)
                if args[0] == "inspect":
                    rows = json.loads(value); rows[0]["HostConfig"]["PidMode"] = mode
                    return json.dumps(rows).encode()
                return value
            with self.assertRaisesRegex(RuntimeError, "private container PID"):
                module.reconcile(IDENTITY, LABELS, request)
            self.assertEqual(docker.writes, [])

    def test_second_stopped_generation_preserves_earlier_valid_marker(self):
        marker = {"version": 1, "authority": "docker-stopped-container", "engineId": "engine-1", "containerId": IDENTITY,
                  "startedAt": "2026-09-26T00:00:00Z", "finishedAt": "2026-09-26T01:00:00Z", "lifetime": RECORD}
        docker = Docker(data=archive([(INSTANCE + ".json", RECORD, tarfile.REGTYPE, 1001),
            (INSTANCE + ".retired.json", RECORD, tarfile.REGTYPE, 1001),
            (INSTANCE + ".container-retired.json", marker, tarfile.REGTYPE, 1001)]))
        self.assertEqual(module.reconcile(IDENTITY, LABELS, docker)["retired"], 0)
        self.assertEqual(docker.writes, [])

    def test_legacy_null_capture_without_any_lifecycle_directory(self):
        docker = Docker()
        entry = {"id": "12345678-1234-1234-1234-123456789012", "bytes": 8, "count": 1, "phase": "exposed", "lifetime": None}
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w") as target:
            raw = json.dumps([entry]).encode(); item = tarfile.TarInfo("ledger.json")
            item.size, item.uid, item.mode = len(raw), 1001, 0o600
            target.addfile(item, io.BytesIO(raw))
        def request(*args, **kwargs):
            if args[:2] == ("system", "dial-stdio"):
                metadata = base64.b64encode(json.dumps({"name": "chariox-browser-uploads-1001", "mode": 0x80000000 | 0o700, "linkTarget": ""}).encode())
                return b"HTTP/1.1 200 OK\r\nX-Docker-Container-Path-Stat: " + metadata + b"\r\n\r\n"
            if args[0] == "cp" and args[-1] == "-":
                if args[1].endswith("/ledger.json"):
                    return output.getvalue()
                raise FileNotFoundError()
            return docker(*args, **kwargs)
        result = module.reconcile(IDENTITY, LABELS, request)
        self.assertEqual(result["legacyEntries"], 1)
        with tarfile.open(fileobj=io.BytesIO(docker.writes[0][1])) as proof:
            self.assertEqual(proof.getmembers()[0].name, "legacy-container-retired.json")
            self.assertEqual(json.load(proof.extractfile(proof.getmembers()[0]))["entries"], [entry])

    def test_legacy_root_stat_rejects_public_or_symlink_directory(self):
        for mode in (0x80000000 | 0o755, 0x08000000 | 0o700, 0o700):
            docker = Docker()
            def request(*args, **kwargs):
                if args[:2] == ("system", "dial-stdio"):
                    value = base64.b64encode(json.dumps({"name": "chariox-browser-uploads-1001", "mode": mode, "linkTarget": ""}).encode())
                    return b"HTTP/1.1 200 OK\r\nX-Docker-Container-Path-Stat: " + value + b"\r\n\r\n"
                return docker(*args, **kwargs)
            with self.assertRaisesRegex(RuntimeError, "private nonsymlink"):
                module.reconcile(IDENTITY, LABELS, request)
            self.assertEqual(docker.writes, [])

    def test_invalid_and_reversed_generation_dates_never_write_proof(self):
        for started, finished in [("2026-99-99T99:99:99Z", "2026-09-27T00:00:00Z"),
                                  ("2026-09-28T00:00:00Z", "2026-09-27T00:00:00Z"),
                                  ("2026-09-27T00:00:00.000000002Z", "2026-09-27T00:00:00.000000001Z")]:
            docker = Docker()
            def request(*args, **kwargs):
                value = docker(*args, **kwargs)
                if args[0] == "inspect":
                    rows = json.loads(value); rows[0]["State"].update(StartedAt=started, FinishedAt=finished)
                    return json.dumps(rows).encode()
                return value
            with self.assertRaisesRegex(RuntimeError, "timestamp"):
                module.reconcile(IDENTITY, LABELS, request)
            self.assertEqual(docker.writes, [])


if __name__ == "__main__":
    unittest.main()
