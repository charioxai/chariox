"""Final fixture restoration controls the verdict and permanently stops probes."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('fixture_lifecycle', Path(__file__).with_name('fixture-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class FixtureCleanupTests(lifecycle.FixtureLifecycleCase):
    def test_successful_cell_restores_each_pointer_probe_and_passes(self):
        code, verdict, result = self.invoke()
        self.assertEqual(code, 0)
        self.assertTrue(verdict.startswith('PASS:'))
        self.assertEqual(len(self.pointer_clicks), 4)
        self.assertEqual(self.warps, [point for name, x, index in self.probes
                                    for point in [{'x': float(x), 'y': 30.0}, self.saved]])
        comparisons = [json.loads(p.read_text()) for p in result.glob('line*.json') if not p.stem.endswith('-warp')]
        self.assertEqual(len(comparisons), 8)
        self.assertTrue(all(row['passed'] for row in comparisons))

    def test_restoration_retirement_fails_evidence_and_stops_even_after_facts_return(self):
        for probe_number in [1, 4]:
            for failure in ['focus', 'pointer', 'window', 'subprocess']:
                with self.subTest(probe_number=probe_number, failure=failure):
                    self.state = {'frontmostPID': 42, 'pointer': self.saved, 'windows': [self.window]}
                    self.warps = []; self.launches = []; self.pointer_clicks = []
                    self.restored_facts = False
                    self.fail_on = probe_number
                    self.restore_failure = failure
                    code, verdict, result = self.invoke()
                    self.assertTrue(self.restored_facts)
                    self.assertEqual(code, 1)
                    self.assertTrue(verdict.startswith('FAIL:'))
                    self.assertEqual(len(self.pointer_clicks), probe_number)
                    self.assertEqual(len(self.launches), 3 + 4 + probe_number)
                    self.assertEqual(len(self.warps), 2 * probe_number - 1)
                    label = self.probes[probe_number - 1][0] + '-pointer'
                    comparison = json.loads((result / f'{label}.json').read_text())
                    self.assertFalse(comparison['passed'])
                    self.assertTrue(comparison['pointerLeaseRetired'])
                    self.assertEqual(self.state['frontmostPID'], 42)
                    self.assertEqual(self.state['windows'], [self.window])


if __name__ == '__main__': unittest.main()
