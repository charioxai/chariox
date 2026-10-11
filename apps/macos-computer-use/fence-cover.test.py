"""First covered-click aborts must close the owned panel or fail cleanup."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('fence_lifecycle', Path(__file__).with_name('fence-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class FenceCoverTests(lifecycle.FenceLifecycleCase):
    def assert_abort(self, report, code):
        self.assertEqual(code, 1)
        self.assertEqual(report['status'], 'FAIL')
        self.assertTrue(report['pointerLeaseRetired'])
        self.assertEqual(self.warps, [])
        self.assertEqual(self.jitters, 0)

    def test_cover_must_close_before_stale_pointer_cell(self):
        self.cover_close_failure = 'sticky'
        code, report = self.invoke()
        self.assertEqual(code, 1)
        self.assertFalse(report['owned_window_closed'])
        self.assertTrue(report['pointerLeaseRetired'])
        self.assertEqual(self.launches[-1], 'nonactivating-cover-pointer')
        self.assertEqual(len(self.warps), 1)
        self.assertEqual(self.jitters, 0)

    def test_reused_cover_pid_is_not_signaled(self):
        self.cover_identity_failure = 'process-after'
        self.fail_label = 'nonactivating-cover-click'; self.helper_failure = 'refusal'
        code, report = self.invoke()
        self.assert_abort(report, code)
        self.assertEqual(self.cover_signals, 0)
        self.assertFalse(report['owned_window_closed'])


def first_check_failure(failure, close_failure):
    def test(self):
        self.fail_label = 'nonactivating-cover-click'; self.helper_failure = failure
        self.cover_close_failure = close_failure
        code, report = self.invoke()
        self.assert_abort(report, code)
        self.assertEqual(self.launches[-1], self.fail_label)
        self.assertGreater(self.cover_signals, 0)
        remaining = any(w['pid'] == 43 for w in self.state['windows'])
        self.assertEqual(remaining, close_failure is not None)
        self.assertEqual(report['owned_window_closed'], not remaining)
    return test


def invalid_identity(failure):
    def test(self):
        self.cover_identity_failure = failure
        code, report = self.invoke()
        self.assert_abort(report, code)
        self.assertEqual(self.launches, ['preflight'])
        self.assertFalse(report['owned_window_closed'])
        if failure in ['missing', 'malformed', 'process', 'zero-pid', 'zero-window', 'launch']:
            self.assertEqual(self.cover_signals, 0)
    return test


for failure in ['refusal', 'mixed', 'success', 'exit', 'timeout', 'identity']:
    for close_failure in [None, 'sticky']:
        setattr(FenceCoverTests, 'test_first_' + failure + '_' + str(close_failure), first_check_failure(failure, close_failure))
for failure in ['permission', 'gone']:
    setattr(FenceCoverTests, 'test_close_' + failure, first_check_failure('refusal', failure))
for failure in ['missing', 'malformed', 'process', 'window-pid', 'window-id', 'zero-pid', 'zero-window', 'launch']:
    setattr(FenceCoverTests, 'test_invalid_' + failure.replace('-', '_'), invalid_identity(failure))

if __name__ == '__main__': unittest.main()
