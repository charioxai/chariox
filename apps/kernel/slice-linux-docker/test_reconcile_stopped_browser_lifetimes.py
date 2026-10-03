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

def head_stat(location, mode):
    metadata = base64.b64encode(json.dumps({"name": location.rsplit("/", 1)[1], "mode": mode, "linkTarget": ""}).encode())
    return b"HTTP/1.1 200 OK\r\nX-Docker-Container-Path-Stat: " + metadata + b"\r\n\r\n"


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
            if ("path=" + module.ROOT + " ").encode() in data:
                return head_stat(module.ROOT, 0x80000000 | 0o700)
            return b"HTTP/1.1 404 Not Found\r\n\r\n"
        if args[0] == "inspect":
            self.reads += 1
            return json.dumps([{"Id": IDENTITY, "Config": {"Labels": LABELS}, "HostConfig": {"PidMode": ""}, "State": {"Status": self.state, "Running": self.state == "running",
                "Paused": self.state == "paused", "Restarting": self.state == "restarting", "Pid": 0,
                "StartedAt": str(self.reads) if self.change else "2026-09-27T00:00:00Z", "FinishedAt": "2026-09-27T01:00:00Z"}}]).encode()
        if args[0] == "cp" and args[-1] == "-":
            return self.data
        if args[:2] == ("cp", "-"):
            self.writes.append((args, data))
            return b""
        raise AssertionError(args)


class ReconciliationTests(unittest.TestCase):
    def test_stopped_named_user_copy_preserves_numeric_archive_owners(self):
        docker = Docker()
        def request(*args, data=None):
            if args[:3] == ("cp", "-a", "-"):
                # Docker 20.10 resolves Config.User through getent for -a,
                # which fails for a stopped container with a named user.
                raise RuntimeError('getent unable to find entry "slice" in passwd database')
            if args[:2] == ("cp", "-"):
                with tarfile.open(fileobj=io.BytesIO(data)) as entries:
                    for entry in entries:
                        self.assertEqual((entry.uid, entry.gid, entry.mode), (1001, 1001, 0o600))
                docker.writes.append((args, data))
                return b""
            return docker(*args, data=data)
        result = module.reconcile(IDENTITY, LABELS, request)
        self.assertEqual(result["retired"], 1)
        self.assertEqual(len(docker.writes), 1)

    def test_missing_lifecycle_uses_head_status_not_docker_cli_error_rendering(self):
        for message in ("No such file or directory", "Could not find the file", "localized error"):
            docker = Docker()
            def request(*args, **kwargs):
                if args[:2] == ("system", "dial-stdio"):
                    return b"HTTP/1.1 404 Not Found\r\n\r\n"
                if args[0] == "cp":
                    raise RuntimeError(message)
                return docker(*args, **kwargs)
            self.assertEqual(module.reconcile(IDENTITY, LABELS, request)["retired"], 0)
            self.assertEqual(docker.writes, [])
            self.assertEqual(docker.reads, 3)

    def test_missing_ledger_uses_its_own_exact_head_without_copy(self):
        docker = Docker()
        def request(*args, **kwargs):
            if args[:2] == ("system", "dial-stdio"):
                if ("path=" + module.UPLOAD_ROOT + " ").encode() in kwargs["data"]:
                    return head_stat(module.UPLOAD_ROOT, 0x80000000 | 0o700)
                return b"HTTP/1.1 404 Not Found\r\n\r\n"
            if args[0] == "cp":
                raise RuntimeError("older Docker CLI: No such file or directory")
            return docker(*args, **kwargs)
        self.assertEqual(module.reconcile(IDENTITY, LABELS, request)["legacyEntries"], 0)
        self.assertEqual(docker.writes, [])

    def test_head_errors_and_malformed_metadata_never_establish_absence(self):
        valid = head_stat(module.ROOT, 0x80000000 | 0o700)
        invalid = [b"404\r\n\r\n", b"HTTP/1.1 403 Forbidden\r\n\r\n",
            b"HTTP/1.1 500 Internal Server Error\r\n\r\n", b"HTTP/1.1 200 OK\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nX-Docker-Container-Path-Stat: not-base64\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nX-Docker-Container-Path-Stat: W10=\r\n\r\n",
            valid + b"unexpected body", valid.replace(b"\r\n\r\n", b"\r\nX-Docker-Container-Path-Stat: W10=\r\n\r\n"),
            head_stat("/wrong-name", 0x80000000 | 0o700), head_stat(module.ROOT, 0o700),
            head_stat(module.ROOT, 0x88000000 | 0o700)]
        for response in invalid:
            with self.subTest(response=response[:60]):
                docker = Docker()
                def request(*args, **kwargs):
                    if args[:2] == ("system", "dial-stdio"):
                        return response
                    return docker(*args, **kwargs)
                with self.assertRaises(RuntimeError):
                    module.reconcile(IDENTITY, LABELS, request)
                self.assertEqual(docker.writes, [])

    def test_copy_failure_after_present_head_is_not_absence(self):
        for error in (FileNotFoundError("exact Docker archive path absent"), RuntimeError("localized CLI error")):
            docker = Docker()
            def request(*args, **kwargs):
                if args[0] == "cp" and args[-1] == "-":
                    raise error
                return docker(*args, **kwargs)
            with self.assertRaises(type(error)):
                module.reconcile(IDENTITY, LABELS, request)
            self.assertEqual(docker.writes, [])

    def test_ledger_head_requires_regular_private_file(self):
        for mode in (0x80000000 | 0o600, 0x08000000 | 0o600, 0o644):
            docker = Docker()
            def request(*args, **kwargs):
                if args[:2] == ("system", "dial-stdio"):
                    if b"/ledger.json " in kwargs["data"]:
                        return head_stat(module.UPLOAD_ROOT + "/ledger.json", mode)
                    if ("path=" + module.UPLOAD_ROOT + " ").encode() in kwargs["data"]:
                        return head_stat(module.UPLOAD_ROOT, 0x80000000 | 0o700)
                    return b"HTTP/1.1 404 Not Found\r\n\r\n"
                return docker(*args, **kwargs)
            with self.assertRaisesRegex(RuntimeError, "private nonsymlink"):
                module.reconcile(IDENTITY, LABELS, request)
            self.assertEqual(docker.writes, [])

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
                if ("path=" + module.ROOT + " ").encode() in kwargs["data"]:
                    return b"HTTP/1.1 404 Not Found\r\n\r\n"
                location = module.UPLOAD_ROOT + "/ledger.json" if b"/ledger.json " in kwargs["data"] else module.UPLOAD_ROOT
                return head_stat(location, 0o600 if location.endswith("ledger.json") else 0x80000000 | 0o700)
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
                if args[:2] == ("system", "dial-stdio") and ("path=" + module.UPLOAD_ROOT + " ").encode() in kwargs["data"]:
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
