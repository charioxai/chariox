"""MP-07 / MP-08 / MP-10: replay the observed midpoint monitor failure."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("monitor", pathlib.Path(__file__).with_name("browser-computer-soak-monitor.py"))
monitor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(monitor)


class RestartMonitorTests(unittest.TestCase):
    def status(self, label="before_restart"):
        return {"updatedAt": "2026-10-01T22:29:16.333Z", "checkpoint": {"label": label, "browserStartTime": "83256867"}}

    def test_planned_restart_does_not_accumulate_false_death_errors(self):
        for timestamp in ["2026-10-01T22:29:20.995299Z", "2026-10-01T22:29:26.012050Z", "2026-10-01T22:29:31.028113Z"]:
            now = monitor.datetime.datetime.fromisoformat(timestamp.replace("Z", "+00:00")).timestamp()
            self.assertEqual(monitor.checkpoint_state(self.status(), None, now), "restarting")

    def test_unplanned_browser_death_still_fails(self):
        now = monitor.datetime.datetime.fromisoformat("2026-10-01T22:29:31+00:00").timestamp()
        with self.assertRaisesRegex(RuntimeError, "outside controlled restart"):
            monitor.checkpoint_state(self.status("periodic"), None, now)

    def test_stuck_restart_has_bounded_deadline(self):
        now = monitor.datetime.datetime.fromisoformat("2026-10-01T22:31:17+00:00").timestamp()
        with self.assertRaisesRegex(RuntimeError, "stale"):
            monitor.checkpoint_state(self.status(), None, now)

    def test_reused_browser_pid_is_rejected(self):
        now = monitor.datetime.datetime.fromisoformat("2026-10-01T22:29:31+00:00").timestamp()
        with self.assertRaisesRegex(RuntimeError, "PID identity changed"):
            monitor.checkpoint_state(self.status(), {"state": "S", "startTime": "different"}, now)

    def test_restarted_browser_resumes_health(self):
        status = self.status("restart")
        status["checkpoint"]["browserStartTime"] = "87579186"
        now = monitor.datetime.datetime.fromisoformat("2026-10-01T22:29:40+00:00").timestamp()
        self.assertEqual(monitor.checkpoint_state(status, {"state": "S", "startTime": "87579186"}, now), "healthy")


if __name__ == "__main__":
    unittest.main()
