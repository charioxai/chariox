"""Helper reference refusals permanently retire the runner's pointer lease."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('lifecycle', Path(__file__).with_name('textedit-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class TextEditReferenceTests(lifecycle.TextEditLifecycleCase):
    def test_reference_failure_with_restored_facts_never_mutates_again(self):
        for label in ['reference', 'line5-before-r-explicit-actual', 'line5-before-r-pointer-actual',
                      'line5-before-r-explicit', 'line5-before-r-pointer']:
            for identity_error in [False, True]:
                with self.subTest(label=label, identity_error=identity_error):
                    self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': []}
                    self.warps = []; self.launches = []
                    self.reference_failure = label; self.reference_error = identity_error
                    code, verdict, _ = self.invoke()
                    self.assertNotEqual(code, 0)
                    self.assertTrue(verdict.startswith('FAIL:'))
                    self.assertEqual(self.launches[-1], label, 'helper failure must prevent all later launches')
                    self.assertNotIn('close', self.launches)
                    self.assertEqual(self.warps, [{'x': 20.0, 'y': 30.0}] if 'pointer' in label else [])
                    self.assertEqual(self.state['windows'], [self.window])
                    self.assertEqual(self.state['frontmostPID'], 42)


if __name__ == '__main__': unittest.main()
