"""Unverified fixture helper launches retire the lease before pointer cleanup."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('fixture_launch_lifecycle', Path(__file__).with_name('fixture-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class FixtureLaunchTests(lifecycle.FixtureLifecycleCase):
    def test_failed_launch_never_restores_or_starts_another_probe(self):
        for failure in ['exit', 'timeout', 'identity']:
            with self.subTest(failure=failure):
                self.state = {'frontmostPID': 42, 'pointer': self.saved, 'windows': [self.window]}
                self.warps = []; self.launches = []; self.pointer_clicks = []
                self.restored_facts = False
                self.fail_on = 1
                self.helper_failure = failure
                code, verdict, result = self.invoke()
                self.assertTrue(self.restored_facts)
                self.assertEqual(code, 1)
                self.assertTrue(verdict.startswith('FAIL:'))
                self.assertEqual(len(self.launches), 8)
                self.assertEqual(self.pointer_clicks, ['line5-before-r-pointer'])
                self.assertEqual(self.warps, [{'x': 20.0, 'y': 30.0}])
                self.assertEqual(self.state['frontmostPID'], 42)
                self.assertEqual(self.state['windows'], [self.window])


if __name__ == '__main__': unittest.main()
