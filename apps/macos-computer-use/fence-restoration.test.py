import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('fence_lifecycle', Path(__file__).with_name('fence-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)

class FenceRestorationTests(lifecycle.FenceLifecycleCase):
    pass


def regression(failure):
    def test(self):
        self.restore_failure = failure
        code, report = self.invoke()
        self.assertTrue(self.restored_facts)
        self.assertEqual(code, 1)
        self.assertEqual(report['status'], 'FAIL')
        self.assertTrue(report['pointerLeaseRetired'])
        self.assertTrue(report['owned_window_closed'])
        self.assertTrue(all(row['passed'] for row in report['checks']))
        self.assertEqual(len(self.warps), 401)
    return test

for failure in ['focus', 'pointer', 'window', 'subprocess']:
    setattr(FenceRestorationTests, 'test_final_' + failure, regression(failure))

if __name__ == '__main__': unittest.main()
