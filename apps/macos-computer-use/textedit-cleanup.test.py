"""The runner ends only its own TextEdit; a retired restoration still fails."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('lifecycle', Path(__file__).with_name('textedit-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)
textedit = lifecycle.textedit


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
                self.assertEqual(self.kills, [], 'takeover makes closure unprovable; the instance is never signalled')
                self.assertIn('owner intervention required', verdict)

    def test_human_input_since_launch_never_signals_the_instance(self):
        self.human_input = True
        code, verdict, checks = self.invoke()
        self.assertNotEqual(code, 0)
        self.assertEqual(self.kills, [])
        self.assertIn('owner intervention required', verdict)
        self.assertEqual(len(self.state['windows']), 1, 'the window stays open for the owner')

    def test_an_additional_user_document_never_signals_the_instance(self):
        for label in ['reference', 'reference-after']:
            with self.subTest(opened_at=label):
                self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': []}
                self.warps = []; self.launches = []; self.kills = []; self.extra_window_at = label
                code, verdict, checks = self.invoke()
                self.assertNotEqual(code, 0)
                self.assertEqual(self.kills, [])
                self.assertIn('owner intervention required', verdict)

    def test_real_input_is_judged_against_hid_idle_since_launch(self):
        with patch.object(textedit.time, 'time', return_value=1000.0):
            with patch.object(textedit, 'hid_idle_seconds', return_value=150.0):
                self.assertFalse(textedit.real_input_since(900.0), 'last input predates the launch')
            with patch.object(textedit, 'hid_idle_seconds', return_value=60.0):
                self.assertTrue(textedit.real_input_since(900.0), 'input 60 s ago, launch 100 s ago')
                self.assertFalse(textedit.real_input_since(990.0), 'input predates a launch 10 s ago')
            with patch.object(textedit, 'hid_idle_seconds', return_value=0.2):
                self.assertTrue(textedit.real_input_since(900.0))


if __name__ == '__main__': unittest.main()
