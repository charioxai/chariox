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
        self.assertEqual(self.launches[-3:], ['same-process-outside-click', 'same-process-outside-pointer', 'same-process-uncovered-capture'])
        self.assertEqual(self.panel_signals, 2)
        self.assertEqual(len([c for c in report['checks'] if c['name'].startswith('same-process')]), 6)


SAME_PROCESS = ['same-process-cover-click', 'same-process-cover-pointer', 'same-process-cover-capture',
                'same-process-outside-click', 'same-process-outside-pointer', 'same-process-uncovered-capture']
# One warp per pointer cell; the stale-pointer jitter adds 400 and precedes every same-process cell.
WARPS = {'nonactivating-cover-pointer': 1, 'stale-pointer': 401, 'same-process-cover-click': 401,
         'same-process-cover-pointer': 402, 'same-process-cover-capture': 402,
         'same-process-outside-click': 402, 'same-process-outside-pointer': 403, 'same-process-uncovered-capture': 403}


class OutsideRefusal(lifecycle.FenceLifecycleCase):
    def test_outside_refusal_fails_the_cell(self):
        self.fail_label = 'same-process-outside-click'; self.helper_failure = 'refusal'
        code, report = self.invoke()
        self.assertEqual(code, 1)
        self.assertEqual([c['name'] for c in report['checks'] if not c['passed']], ['same-process-outside-click'])


def regression(label, failure):
    def test(self):
        self.fail_label = label; self.helper_failure = failure
        code, report = self.invoke()
        self.assertTrue(self.restored_facts)
        self.assertEqual(code, 1)
        self.assertEqual(report['status'], 'FAIL')
        self.assertEqual(self.launches[-1], label)
        expected_jitters = 1 if label in SAME_PROCESS or label == 'stale-pointer' else 0
        self.assertEqual(self.jitters, expected_jitters)
        self.assertEqual(len(self.warps), WARPS.get(label, 0))
    return test

for label in ['preflight', 'nonactivating-cover-click', 'nonactivating-cover-pointer', 'stale-pointer'] + SAME_PROCESS:
    for failure in ['refusal', 'mixed', 'success', 'exit', 'timeout', 'identity']:
        if label == 'preflight' and failure in ['refusal', 'mixed', 'success']: continue
        # Allowed cells never abort on a bad receipt; see test_outside_refusal_fails_the_cell.
        if label in SAME_PROCESS[3:] and failure in ['refusal', 'mixed', 'success']: continue
        setattr(FenceLaunchTests, 'test_' + label.replace('-', '_') + '_' + failure, regression(label, failure))



def panel_failure(failure):
    def test(self):
        self.panel_failure = failure
        code, report = self.invoke()
        self.assertEqual(code, 1)
        self.assertEqual(report['status'], 'FAIL')
        # A stuck panel is only noticed after the allowed cells; the others abort at the panel itself.
        self.assertEqual(self.launches[-1], 'same-process-outside-pointer' if failure == 'sticky' else 'stale-pointer')
        self.assertFalse(any(w['pid'] == 42 for w in self.state['windows']))
    return test

for failure in ['silent', 'below', 'foreign-pid', 'sticky']:
    setattr(FenceLaunchTests, 'test_panel_' + failure.replace('-', '_'), panel_failure(failure))

if __name__ == '__main__': unittest.main()
