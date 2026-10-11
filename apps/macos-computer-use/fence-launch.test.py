import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('fence_lifecycle', Path(__file__).with_name('fence-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)

class FenceLaunchTests(lifecycle.FenceLifecycleCase):
    def test_expected_refusals_preserve_lease_and_pass(self):
        code, report = self.invoke()
        self.assertEqual(code, 0)
        self.assertEqual(report['status'], 'PASS')
        self.assertEqual(self.jitters, 1)
        self.assertEqual(self.warps[-1], self.saved)


def regression(label, failure):
    def test(self):
        self.fail_label = label; self.helper_failure = failure
        code, report = self.invoke()
        self.assertTrue(self.restored_facts)
        self.assertEqual(code, 1)
        self.assertEqual(report['status'], 'FAIL')
        self.assertEqual(self.launches[-1], label)
        expected_jitters = 1 if label == 'stale-pointer' else 0
        self.assertEqual(self.jitters, expected_jitters)
        self.assertEqual(len(self.warps), 401 if expected_jitters else (1 if label == 'nonactivating-cover-pointer' else 0))
    return test

for label in ['preflight', 'nonactivating-cover-click', 'nonactivating-cover-pointer', 'stale-pointer']:
    for failure in ['refusal', 'mixed', 'success', 'exit', 'timeout', 'identity']:
        if label == 'preflight' and failure in ['refusal', 'mixed', 'success']: continue
        setattr(FenceLaunchTests, 'test_' + label.replace('-', '_') + '_' + failure, regression(label, failure))

if __name__ == '__main__': unittest.main()
