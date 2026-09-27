import importlib.util
import io
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


if __name__ == "__main__":
    unittest.main()
