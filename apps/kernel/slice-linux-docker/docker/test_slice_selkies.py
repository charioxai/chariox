#!/usr/bin/env python3
"""Focused tests for the private Selkies lifecycle."""

import importlib.util
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("slice-selkies.py")
SPEC = importlib.util.spec_from_file_location("slice_selkies_lifecycle", SCRIPT)
LIFECYCLE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LIFECYCLE)


class FakeProcess:
    def __init__(self):
        self.terminated = False
        self.killed = False

    def terminate(self):
        self.terminated = True

    def kill(self):
        self.killed = True


class SelkiesStopTests(unittest.TestCase):
    def test_new_records_use_stable_identity_and_ignore_timestamp_for_generation(self):
        record = LIFECYCLE.process_record(LIFECYCLE.psutil.Process())
        self.assertIsNotNone(LIFECYCLE.owned_process(record))
        self.assertEqual(LIFECYCLE.process_key(record), LIFECYCLE.process_key({**record, "created": 0}))
        self.assertNotEqual(LIFECYCLE.process_key(record),
                            LIFECYCLE.process_key({**record, "start_ticks": record["start_ticks"] + 1}))

    def test_proc_stat_parser_handles_parentheses_in_process_name(self):
        stat = "17 (selkies (worker)) S " + " ".join(["0"] * 18 + ["12345"])
        with mock.patch.object(Path, "read_text", side_effect=["boot-id\n", stat]):
            self.assertEqual(LIFECYCLE.process_start_identity(17), {"boot_id": "boot-id", "start_ticks": 12345})

    def test_wrong_user_zombie_and_partial_identity_are_not_owned(self):
        record = LIFECYCLE.process_record(LIFECYCLE.psutil.Process())
        with mock.patch.object(LIFECYCLE.psutil.Process, "uids", return_value=SimpleNamespace(real=os.getuid() + 1)):
            self.assertIsNone(LIFECYCLE.owned_process(record))
        with mock.patch.object(LIFECYCLE.psutil.Process, "status", return_value=LIFECYCLE.psutil.STATUS_ZOMBIE):
            self.assertIsNone(LIFECYCLE.owned_process(record))
        incomplete = {key: value for key, value in record.items() if key != "boot_id"}
        self.assertIsNone(LIFECYCLE.owned_process(incomplete))

    def test_legacy_records_keep_strict_identity_without_timestamp_tolerance(self):
        process = LIFECYCLE.psutil.Process()
        record = {"pid": process.pid, "created": process.create_time()}
        self.assertIsNotNone(LIFECYCLE.owned_process(record))
        self.assertIsNone(LIFECYCLE.owned_process({**record, "created": record["created"] + 1}))

    def test_owned_process_survives_wall_clock_creation_time_shift(self):
        process = LIFECYCLE.psutil.Process()
        record = {"pid": process.pid, "created": process.create_time(),
                  "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                  "start_ticks": int(Path(f"/proc/{process.pid}/stat").read_text().rpartition(")")[2].split()[19])}
        with mock.patch.object(LIFECYCLE.psutil.Process, "create_time", return_value=record["created"] - 1):
            self.assertIsNotNone(LIFECYCLE.owned_process(record))

    def test_stable_identity_rejects_pid_reuse_and_boot_change(self):
        process = LIFECYCLE.psutil.Process()
        record = {"pid": process.pid, "created": process.create_time(),
                  "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                  "start_ticks": int(Path(f"/proc/{process.pid}/stat").read_text().rpartition(")")[2].split()[19])}
        self.assertIsNone(LIFECYCLE.owned_process({**record, "start_ticks": record["start_ticks"] + 1}))
        self.assertIsNone(LIFECYCLE.owned_process({**record, "boot_id": "other-boot"}))

    def test_stop_accepts_a_process_that_becomes_unowned_after_sigterm(self):
        process = FakeProcess()
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch)
            LIFECYCLE.write_state(directory, {"pid": 17, "created": 1.0})
            with mock.patch.object(LIFECYCLE, "owned_process", side_effect=[process, None]):
                result = LIFECYCLE.stop(directory)

            self.assertEqual(result, {"stopped": True, "forced": False})
            self.assertTrue(process.terminated)
            self.assertFalse(process.killed)
            self.assertFalse((directory / "process.json").exists())

    def test_stop_reports_when_forced_termination_was_required(self):
        process = FakeProcess()
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch)
            LIFECYCLE.write_state(directory, {"pid": 18, "created": 2.0})
            with mock.patch.object(LIFECYCLE, "owned_process", return_value=process), mock.patch.object(
                LIFECYCLE,
                "wait_until_not_owned",
                side_effect=[False, True],
            ):
                result = LIFECYCLE.stop(directory)

            self.assertEqual(result, {"stopped": True, "forced": True})
            self.assertTrue(process.terminated)
            self.assertTrue(process.killed)
            self.assertFalse((directory / "process.json").exists())


if __name__ == "__main__":
    unittest.main()
