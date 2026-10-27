#!/usr/bin/env python3
"""MP-11: prove the validation watchdog cannot signal special/reused PIDs."""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("validation", Path(__file__).with_name("kernel-access-validation.py"))
validation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validation)


class WatchdogTests(unittest.TestCase):
    def test_special_or_invalid_targets_never_reach_pidfd(self):
        for pid in [0, 1, -1, None, float("nan"), 2147483648]:
            with self.subTest(pid=pid), patch.object(validation, "collect_owned"), patch.object(validation.os, "pidfd_open") as opened:
                if pid == 2147483648:
                    self.assertIsNone(validation.identity(pid))
                    validation.stop_owned({pid: 0})
                else:
                    self.assertIsNone(validation.identity(pid))
                    with self.assertRaises(RuntimeError):
                        validation.stop_owned({pid: 0})
                opened.assert_not_called()

    def test_real_stop_path_terminates_only_matching_birth(self):
        child = subprocess.Popen(["/bin/sleep", "30"])
        birth = validation.identity(child.pid)[1]
        try:
            validation.stop_owned({child.pid: birth + 1})
            self.assertIsNone(child.poll(), "stale identity must not signal the process")
            validation.stop_owned({child.pid: birth})
            self.assertLess(child.wait(timeout=3), 0)
        finally:
            validation.stop_owned({child.pid: birth})
            child.wait(timeout=3)


if __name__ == "__main__":
    unittest.main()
