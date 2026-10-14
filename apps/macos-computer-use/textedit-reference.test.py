"""Helper capture/click refusals permanently retire the runner's pointer lease."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('lifecycle', Path(__file__).with_name('textedit-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class TextEditReferenceTests(lifecycle.TextEditLifecycleCase):
    def test_reference_failure_with_restored_facts_never_mutates_again(self):
        for label in ['reference', 'line5-before-r-explicit', 'line5-before-r-pointer', 'reference-after']:
            for identity_error in [False, True]:
                with self.subTest(label=label, identity_error=identity_error):
                    self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': []}
                    self.warps = []; self.launches = []; self.kills = []
                    self.reference_failure = label; self.reference_error = identity_error
                    code, verdict, _ = self.invoke()
                    self.assertNotEqual(code, 0)
                    self.assertTrue(verdict.startswith('FAIL:'))
                    self.assertEqual(self.launches[-1], label, 'helper failure must prevent all later launches')
                    warps = {'line5-before-r-pointer': [{'x': 20.0, 'y': 30.0}],
                             'reference-after': [{'x': 20.0, 'y': 30.0}, {'x': 40.0, 'y': 30.0}]}
                    self.assertEqual(self.warps, warps.get(label, []))
                    self.assertEqual(self.kills, [], 'a reported takeover makes closure unprovable; never signalled')
                    self.assertIn('owner intervention required', verdict)
                    self.assertEqual(self.state['frontmostPID'], 42)


if __name__ == '__main__': unittest.main()
