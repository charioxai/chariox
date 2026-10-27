import importlib.util
import ast
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("admission", Path(__file__).with_name("check-browser-overlay-admission.py"))
admission = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(admission)


class BrowserOverlayAdmissionTests(unittest.TestCase):
    def census_function(self, name):
        function = next(node for node in ast.parse(admission.CENSUS).body if isinstance(node, ast.FunctionDef) and node.name == name)
        namespace = {'os': os, 'stat': stat, 'json': json}
        exec(compile(ast.Module(body=[function], type_ignores=[]), '<census>', 'exec'), namespace)
        return namespace[name]

    def test_both_browser_profile_argument_forms_are_detected(self):
        matches = self.census_function('profile_arguments')
        self.assertTrue(matches(b'chromium\0--user-data-dir=/tmp/profile\0', '/tmp/profile'))
        self.assertTrue(matches(b'chromium\0--user-data-dir\0/tmp/profile\0', '/tmp/profile'))
        self.assertFalse(matches(b'chromium\0--user-data-dir=/tmp/profile-other\0', '/tmp/profile'))
        self.assertFalse(matches(b'chromium\0--user-data-dir\0', '/tmp/profile'))

    def test_record_read_is_bounded_private_and_rejects_symlink(self):
        read = self.census_function('read_record')
        with tempfile.TemporaryDirectory(prefix='browser-overlay-record-') as directory:
            pointer = Path(directory) / 'record.json'
            pointer.write_text('{"version":1}')
            pointer.chmod(0o600)
            self.assertEqual(read(pointer, os.getuid()), {'version': 1})
            pointer.write_bytes(b'x' * 4097)
            with self.assertRaises(ValueError):
                read(pointer, os.getuid())
            link = Path(directory) / 'link'
            link.symlink_to(pointer)
            with self.assertRaises(OSError):
                read(link, os.getuid())

    def test_record_growth_and_named_replacement_are_rejected(self):
        read = self.census_function('read_record')
        with tempfile.TemporaryDirectory(prefix='browser-overlay-record-race-') as directory:
            pointer = Path(directory) / 'record.json'
            other = Path(directory) / 'replacement.json'
            for file in (pointer, other):
                file.write_text('{"version":1}')
                file.chmod(0o600)
            replacement = other.stat()
            with patch.object(Path, 'lstat', return_value=replacement):
                with self.assertRaises(ValueError):
                    read(pointer, os.getuid())
            original = os.fstat
            first = True
            def grow(fd):
                nonlocal first
                result = original(fd)
                if first:
                    first = False
                    with pointer.open('ab') as stream:
                        stream.write(b' ' * 5000)
                return result
            with patch.object(os, 'fstat', side_effect=grow):
                with self.assertRaises(ValueError):
                    read(pointer, os.getuid())

    def fixture(self, browser="unowned"):
        calls = []
        info = {"Id": "a" * 64, "State": {"Running": True, "Paused": False, "Restarting": False,
                "Status": "running", "Pid": 123, "StartedAt": "start", "FinishedAt": ""},
                "HostConfig": {"PidMode": "", "RestartPolicy": {"Name": "no"}}, "Config": {"Labels": {
                    "io.chariox.slice.id": "slice-1", "io.chariox.slice.owner-kernel-id": "kernel-1",
                    "io.chariox.slice.owner-machine-id": "machine-1"}}}
        observation = {"disposition": browser, "profileProcessCount": 1 if browser != "clear" else 0}
        def run(args):
            calls.append(args)
            if args[0] == "info":
                return "engine-1"
            if args[0] == "inspect":
                return json.dumps([info])
            if args[0] == "exec":
                return json.dumps(observation)
            raise AssertionError("unexpected mutation")
        options = {"container": "slice-name", "slice_id": "slice-1", "owner_kernel_id": "kernel-1",
                   "owner_machine_id": "machine-1", "profile": "/home/slice/.chariox/browser/chromium", "uid": 1001}
        return options, info, observation, calls, run

    def test_legacy_refuses_before_overlay_without_stopping_any_process(self):
        options, _, _, calls, run = self.fixture()
        overlay = []
        with self.assertRaisesRegex(RuntimeError, "normal kernel.*stop.*start"):
            admission.check(options, run)
            overlay.append("mutated")
        self.assertEqual(overlay, [])
        self.assertTrue(all(command[0] in ("info", "inspect", "exec") for command in calls))

    def test_normal_stopped_boundary_then_clean_start_admits_overlay(self):
        options, info, observation, _, run = self.fixture()
        info["State"].update(Running=False, Status="exited", Pid=0, FinishedAt="finish")
        self.assertEqual(admission.check(options, run)["disposition"], "stopped")
        info["State"].update(Running=True, Status="running", Pid=456, StartedAt="new-start", FinishedAt="finish")
        observation.update(disposition="clear", profileProcessCount=0)
        self.assertEqual(admission.check(options, run)["disposition"], "clear")

    def test_owned_current_browser_is_not_forced_to_restart(self):
        options, _, _, calls, run = self.fixture("owned")
        self.assertEqual(admission.check(options, run)["disposition"], "owned")
        self.assertFalse(any(command[0] in ("stop", "kill", "restart") for command in calls))

    def test_existing_container_profile_override_is_observed(self):
        options, info, _, calls, run = self.fixture("clear")
        options['profile'] = None
        info['Config']['Env'] = ['HOME=/home/slice', 'CHARIOX_SLICE_CHROME_PROFILE=/home/slice/custom-profile']
        admission.check(options, run)
        command = next(command for command in calls if command[0] == 'exec')
        self.assertEqual(command[-2], '/home/slice/custom-profile')

    def test_wrong_owner_shared_pid_or_generation_change_refuses(self):
        for mutate in [lambda info: info["Config"]["Labels"].update({"io.chariox.slice.id": "foreign"}),
                       lambda info: info["HostConfig"].update(PidMode="host")]:
            options, info, _, _, run = self.fixture("clear")
            mutate(info)
            with self.assertRaises(RuntimeError):
                admission.check(options, run)
        options, info, _, _, run = self.fixture("clear")
        def changed(args):
            result = run(args)
            if args[0] == "exec":
                info["State"]["StartedAt"] = "replacement-generation"
            return result
        with self.assertRaisesRegex(RuntimeError, "changed"):
            admission.check(options, changed)


if __name__ == "__main__":
    unittest.main()
