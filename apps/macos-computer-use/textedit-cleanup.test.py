"""The runner ends only its own TextEdit; a retired restoration still fails."""
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
        self.assertEqual(self.launches[-1], 'reference-after')
        self.assertEqual(self.kills, [42])
        self.assertEqual(self.state['windows'], [])

    def test_retired_final_restore_fails_without_native_mutation(self):
        for failure in ['focus', 'pointer', 'subprocess']:
            with self.subTest(failure=failure):
                self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': []}
                self.warps = []; self.launches = []; self.kills = []; self.restore_failure = failure
                code, verdict, checks = self.invoke()
                self.assertEqual(len(checks), 6, 'all four comparisons must complete before the takeover')
                self.assertEqual(self.launches[-1], 'reference-after')
                self.assertEqual(len(self.warps), 2, 'no restoration warp after retirement')
                self.assertNotEqual(code, 0)
                self.assertIn('FAIL: pointer lease retired', verdict)
                self.assertEqual(self.kills, [42], 'the runner-owned instance still ends')


if __name__ == '__main__': unittest.main()
