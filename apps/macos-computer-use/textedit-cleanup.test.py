"""Retired final restoration cannot report successful TextEdit cleanup."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('lifecycle', Path(__file__).with_name('textedit-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class TextEditCleanupTests(lifecycle.TextEditLifecycleCase):
    def test_four_comparisons_and_cleanup_pass(self):
        code, verdict, checks = self.invoke()
        self.assertEqual(code, 0)
        self.assertTrue(verdict.startswith('PASS:'))
        self.assertEqual(len(checks), 6)  # Four comparisons plus two warp receipts.
        self.assertEqual(self.launches[-1], 'close')
        self.assertEqual(self.state['windows'], [])

    def test_retired_final_restore_fails_without_close_mutation(self):
        for failure in ['focus', 'pointer', 'subprocess']:
            with self.subTest(failure=failure):
                self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': []}
                self.warps = []; self.launches = []; self.restore_failure = failure
                code, verdict, checks = self.invoke()
                self.assertEqual(len(checks), 6, 'all four comparisons must complete before the takeover')
                self.assertNotIn('close', self.launches)
                self.assertEqual(len(self.warps), 2, 'no restoration warp after retirement')
                self.assertNotEqual(code, 0)
                self.assertIn('FAIL: cleanup incomplete:', verdict)
                self.assertEqual(self.state['windows'], [self.window])


if __name__ == '__main__': unittest.main()
