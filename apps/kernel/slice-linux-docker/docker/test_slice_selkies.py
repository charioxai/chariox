#!/usr/bin/env python3
"""Focused tests for the private Selkies lifecycle."""

import importlib.util
from pathlib import Path
import tempfile
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
