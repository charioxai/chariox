import importlib.util
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import uuid

HERE = Path(__file__).parent


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@unittest.skipUnless(sys.platform == "linux", "requires real Linux procfs, pidfd and subreaper")
class BrowserLifetimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="chariox-browser-lifetime-test-")
        self.root = Path(self.temp.name)
        self.old_env = dict(os.environ)
        os.environ["CHARIOX_BROWSER_LIFECYCLE_ROOT"] = str(self.root / "lifetimes")
        os.environ["TMPDIR"] = str(self.root)
        self.lifecycle = load("lifecycle_test", "browser-lifecycle.py")
        self.store = load("store_test", "browser-upload-store.py")
        self.staging = self.root / f"chariox-browser-uploads-{os.getuid()}"
        self.profile = str(self.root / "profile")
        self.processes = []

    def tearDown(self):
        try:
            subprocess.run([sys.executable, str(HERE / "browser-lifecycle.py"), "stop", self.profile],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
            for process in self.processes:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=3)
        finally:
            os.environ.clear()
            os.environ.update(self.old_env)
            self.temp.cleanup()

    def wait_file(self, path):
        deadline = time.monotonic() + 5
        while not path.exists():
            self.assertLess(time.monotonic(), deadline, f"timed out waiting for {path.name}")
            time.sleep(0.02)
        return path.read_text()

    def start_browser(self):
        port_file = self.root / "port"
        escaped_file = self.root / "escaped"
        code = f'''import os, socket, subprocess, sys, time
s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen()
open({str(port_file)!r}, "w").write(str(s.getsockname()[1]))
p = subprocess.Popen([sys.executable, "-c", "import os,time; os.setsid(); time.sleep(60)"])
open({str(escaped_file)!r}, "w").write(str(p.pid))
time.sleep(60)
'''
        result = subprocess.run([sys.executable, str(HERE / "browser-lifecycle.py"), "start", self.profile,
                                 str(self.root / "browser.log"), sys.executable, "-c", code,
                                 f"--user-data-dir={self.profile}"],
                                text=True, capture_output=True, timeout=8)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout), int(self.wait_file(port_file)), int(self.wait_file(escaped_file))

    def reserve(self, **extra):
        entry = {"id": str(uuid.uuid4()), "browser": "a" * 64, "bytes": 8, "count": 1}
        self.store.transact(self.staging, {"action": "reserve", "entry": entry, "maximumBytes": 8, "maximumFiles": 1, **extra})
        directory = self.staging / entry["id"]
        directory.mkdir(mode=0o700)
        (directory / "file.txt").write_text("approved")
        return entry

    def test_retirement_reaps_escaped_descendants_and_preserves_unrelated_group(self):
        unowned = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)",
                                    f"--user-data-dir={self.profile}"], start_new_session=True)
        self.processes.append(unowned)
        self.assertIn(unowned.pid, self.lifecycle.profile_processes(self.profile))
        forged = {**self.lifecycle.identity(unowned.pid), "start": "0"}
        self.assertIn(unowned.pid, self.lifecycle.profile_processes(self.profile, forged))
        with self.assertRaisesRegex(RuntimeError, "unowned browser process"):
            self.lifecycle.start(self.profile, str(self.root / "browser.log"), [sys.executable, "-c", "pass"])
        unowned.terminate()
        unowned.wait(timeout=3)
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], start_new_session=True)
        self.processes.append(unrelated)
        record, port, escaped = self.start_browser()
        entry = self.reserve(browserPid=record["browser"]["pid"], browserPort=port)
        self.store.transact(self.staging, {"action": "expose", "id": entry["id"]})
        self.store.transact(self.staging, {"action": "reap"})
        self.assertTrue((self.staging / entry["id"] / "file.txt").exists())
        self.assertFalse(self.store.retired(record))
        result = subprocess.run([sys.executable, str(HERE / "browser-lifecycle.py"), "stop", self.profile],
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.store.retired(record))
        self.assertFalse(Path(f"/proc/{escaped}").exists())
        self.assertFalse(Path(f"/proc/{record['browser']['pid']}").exists())
        self.assertIsNone(unrelated.poll())
        self.assertFalse((self.staging / entry["id"]).exists())
        self.assertEqual(json.loads((self.staging / "ledger.json").read_text()), [])

    def test_leader_exit_is_not_retirement_while_escaped_child_lives(self):
        record, _port, escaped = self.start_browser()
        os.kill(record["browser"]["pid"], signal.SIGKILL)
        time.sleep(0.15)
        self.assertTrue(Path(f"/proc/{escaped}").exists())
        self.assertFalse(self.store.retired(record))
        self.assertTrue(self.lifecycle.same_process(record["supervisor"]))

    def test_graceful_close_ack_waits_for_owned_browser_flush(self):
        # MP-08/MP-10/MP-11: CDP acknowledges close before profile writes and
        # the owned descendant lifetime finish. TERM must not race that flush.
        closing = self.root / "closing"
        flushed = self.root / "flushed"
        ready = self.root / "flush-ready"
        code = f'''import pathlib,time
pathlib.Path({str(ready)!r}).touch()
while not pathlib.Path({str(closing)!r}).exists(): time.sleep(.01)
time.sleep(.2)
pathlib.Path({str(flushed)!r}).touch()
'''
        started = subprocess.run([sys.executable, str(HERE / "browser-lifecycle.py"), "start", self.profile,
                                  str(self.root / "browser.log"), sys.executable, "-c", code,
                                  f"--user-data-dir={self.profile}"],
                                 text=True, capture_output=True, timeout=8)
        self.assertEqual(started.returncode, 0, started.stderr)
        record = json.loads(started.stdout)
        self.wait_file(ready)
        actual_run = subprocess.run

        def acknowledge_close(command, **kwargs):
            if command[0] == "node":
                closing.touch()
                return subprocess.CompletedProcess(command, 0)
            return actual_run(command, **kwargs)

        with patch.object(self.lifecycle.subprocess, "run", side_effect=acknowledge_close):
            self.lifecycle.stop_locked(self.lifecycle.directory(), self.profile)
        self.assertTrue(flushed.exists(), "close acknowledgement must allow profile flush before TERM")
        self.assertTrue(self.store.retired(record))

    def test_reused_or_cross_namespace_identity_is_not_live_ownership(self):
        owner = self.lifecycle.identity(os.getpid())
        self.assertEqual(self.store.process_state({**owner, "start": "0"}), "dead")
        self.assertEqual(self.store.process_state({**owner, "namespace": "other"}), "unknown")
        self.assertEqual(self.store.process_state({**owner, "group": owner["group"] + 1}), "unknown")
        self.assertFalse(self.store.retired({"instance": uuid.uuid4().hex}))

    def test_stop_failure_does_not_claim_retirement_or_target_reused_identity(self):
        record, port, _escaped = self.start_browser()
        self.assertIsNone(self.store.browser_owner(os.getpid(), port))
        pointer = self.lifecycle.current_path(self.lifecycle.directory(), self.profile)
        wrong = {**record, "supervisor": {**record["supervisor"], "start": "0"}}
        self.lifecycle.write_json(pointer, wrong)
        try:
            with self.assertRaisesRegex(RuntimeError, "identity is gone"):
                self.lifecycle.stop_locked(self.lifecycle.directory(), self.profile, settlement_timeout=0.05)
            self.assertTrue(self.lifecycle.same_process(record["browser"]))
        finally:
            self.lifecycle.write_json(pointer, record)
        os.kill(record["supervisor"]["pid"], signal.SIGSTOP)
        try:
            with self.assertRaisesRegex(RuntimeError, "did not settle"):
                self.lifecycle.stop_locked(self.lifecycle.directory(), self.profile, settlement_timeout=0.05)
            self.assertFalse(self.store.retired(record))
            self.assertTrue(self.lifecycle.same_process(record["browser"]))
        finally:
            os.kill(record["supervisor"]["pid"], signal.SIGCONT)

    def test_dead_controller_reservation_is_reclaimed_but_exposed_bytes_are_retained(self):
        record, port, _escaped = self.start_browser()
        for exposed in (False, True):
            instance = str(uuid.uuid4())
            ready = self.root / f"ready-{exposed}"
            request = {"action": "reserve", "entry": {"id": instance, "browser": "a" * 64, "bytes": 8, "count": 1}, "maximumBytes": 8, "maximumFiles": 1,
                       "browserPid": record["browser"]["pid"], "browserPort": port}
            code = f'''import json, pathlib, subprocess, sys, time
def tx(value):
 p = subprocess.run([sys.executable, {str(HERE / 'browser-upload-store.py')!r}, {str(self.staging)!r}], input=json.dumps(value), text=True, capture_output=True)
 if p.returncode: raise RuntimeError(p.stderr)
tx({request!r})
p = pathlib.Path({str(self.staging / instance)!r}); p.mkdir(); (p / "file").write_text("approved")
if {exposed!r}: tx({{'action': 'expose', 'id': {instance!r}}})
pathlib.Path({str(ready)!r}).write_text("ready")
time.sleep(60)
'''
            controller = subprocess.Popen([sys.executable, "-c", code])
            self.processes.append(controller)
            self.wait_file(ready)
            controller.kill(); controller.wait(timeout=3)
            self.store.transact(self.staging, {"action": "reap"})
            self.assertEqual((self.staging / instance).exists(), exposed)

    def test_killed_mutator_releases_flock_without_a_stale_directory_lock(self):
        ready = self.root / "mutator-ready"
        code = f'''import importlib.util, pathlib, time
s = importlib.util.spec_from_file_location("store", {str(HERE / 'browser-upload-store.py')!r}); m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
def hang(*args):
 pathlib.Path({str(ready)!r}).write_text("ready")
 time.sleep(60)
m.lifecycle.write_json = hang
m.transact(pathlib.Path({str(self.staging)!r}), {{"action":"reap"}})
'''
        mutator = subprocess.Popen([sys.executable, "-c", code])
        self.processes.append(mutator)
        self.wait_file(ready)
        with self.assertRaises(BlockingIOError):
            self.store.transact(self.staging, {"action": "reap"})
        mutator.kill(); mutator.wait(timeout=3)
        self.store.transact(self.staging, {"action": "reap"})
        self.assertEqual(json.loads((self.staging / "ledger.json").read_text()), [])

    def test_start_acknowledges_owned_browser_when_upload_reaper_is_contended(self):
        self.staging.mkdir(mode=0o700)
        with open(self.staging / "quota.lock", "w") as lock:
            os.chmod(lock.name, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            record, _port, _escaped = self.start_browser()
            self.assertTrue(self.lifecycle.same_process(record["browser"]))

    def test_launch_waits_for_short_lived_lock_holder(self):
        root = self.lifecycle.directory()
        ready = self.root / "launch-lock-ready"
        holder = subprocess.Popen([sys.executable, "-c", f'''import fcntl,time,pathlib
with open({str(root / "launch.lock")!r}, "w") as f:
 fcntl.flock(f,fcntl.LOCK_EX); pathlib.Path({str(ready)!r}).touch(); time.sleep(.2)
'''])
        self.processes.append(holder)
        self.wait_file(ready)
        record, _port, _escaped = self.start_browser()
        self.assertTrue(self.lifecycle.same_process(record["browser"]))


if __name__ == "__main__":
    unittest.main()
