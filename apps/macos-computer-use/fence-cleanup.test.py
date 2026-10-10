import unittest
from fence_cleanup import finish_cleanup

class CleanupTests(unittest.TestCase):
    def test_cleanup_failure_overrides_pass(self):
        clock = [0]
        def wait(seconds): clock[0] += seconds
        report = {'status': 'PASS'}
        finish_cleanup(report, lambda: False, now=lambda: clock[0], wait=wait)
        self.assertEqual(report['status'], 'FAIL')
        self.assertFalse(report['owned_window_closed'])
    def test_cleanup_waits_for_windowserver(self):
        clock = [0]
        def wait(seconds): clock[0] += seconds
        report = {'status': 'PASS'}
        finish_cleanup(report, lambda: clock[0] >= .2, now=lambda: clock[0], wait=wait)
        self.assertEqual(report['status'], 'PASS')
        self.assertTrue(report['owned_window_closed'])
if __name__ == '__main__': unittest.main()
