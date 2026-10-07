"""MP-08 / MP-10 / MP-11: runner admission and attribution checks."""
import unittest
from contract import validate_lock, completed_answer, admit_usage, admit_token_usage, safe_pid, quota_exhausted


class ContractTests(unittest.TestCase):
    def test_full_denominators_are_required(self):
        import json
        from pathlib import Path
        lock = json.loads(Path(__file__).with_name('inputs.lock.json').read_text())
        validate_lock(lock)
        lock['terminal_bench_2']['task_ids'].pop()
        with self.assertRaises(ValueError):
            validate_lock(lock)

    def test_never_signal_system_or_invalid_pid(self):
        for pid in [None, 0, 1, -1, -10, float('nan'), True, '22']:
            with self.assertRaises(ValueError):
                safe_pid(pid)
        self.assertEqual(safe_pid(22), 22)

    def test_final_is_bound_to_submitted_agent_and_prompt(self):
        snapshot = {'session': {'id': 's', 'agents': [{'id': 'a', 'state': 'Idle'}]},
                    'transcript': {'visibleAgentId': 'a', 'entries': [{'role': 'assistant', 'promptId': 'old', 'text': 'wrong'},
                                               {'role': 'assistant', 'promptId': 'p', 'text': 'right'}]}}
        self.assertEqual(completed_answer(snapshot, 's', 'a', 'p'), 'right')
        snapshot['session']['agents'][0]['state'] = 'Working'
        self.assertIsNone(completed_answer(snapshot, 's', 'a', 'p'))
        snapshot['session']['agents'][0]['state'] = 'Idle'
        self.assertIsNone(completed_answer(snapshot, 's', 'other', 'p'))

    def test_only_fresh_active_plan_exhaustion_stops_runs(self):
        meter = {'kind': 'rolling_limit', 'state': 'exhausted', 'observed_at_ms': 1000, 'resets_at_ms': 5000}
        self.assertTrue(quota_exhausted([meter], 2000))
        self.assertFalse(quota_exhausted([meter], 6000))
        self.assertFalse(quota_exhausted([dict(meter, kind='credits')], 2000))
        self.assertFalse(quota_exhausted([dict(meter, observed_at_ms=0)], 400000))

    def test_completed_tui_display_state_is_done(self):
        snapshot = {'session': {'id': 's', 'agents': [{'id': 'a', 'state': 'Done'}]},
                    'transcript': {'visibleAgentId': 'a', 'entries': [
                        {'role': 'assistant', 'promptId': 'p', 'text': 'answer'}]}}
        self.assertEqual(completed_answer(snapshot, 's', 'a', 'p'), 'answer')
        snapshot['session']['agents'][0]['state'] = 'Error'
        self.assertIsNone(completed_answer(snapshot, 's', 'a', 'p'))

    def test_known_tokens_do_not_require_an_invented_price(self):
        report = dict(session_id='s', complete=True, input_tokens=10, cached_input_tokens=2,
                      output_tokens=3, api_equivalent_nanodollars=None)
        self.assertEqual(admit_token_usage(report, 's'), report)
        with self.assertRaises(ValueError):
            admit_usage(report, 's')

    def test_missing_accounting_is_not_zero(self):
        with self.assertRaises(ValueError):
            admit_usage({}, 's')
        with self.assertRaises(ValueError):
            admit_usage({'session_id': 'other', 'complete': True}, 's')


if __name__ == '__main__':
    unittest.main()
