"""MP-08 / MP-10 / MP-11: setup retry cannot reuse provider or verifier feedback."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from terminal_campaign import resume_setup_campaign


class SetupResumeTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix='chariox-evals-setup-test-')
        self.addCleanup(self.scratch.cleanup)
        self.receipt = Path(self.scratch.name) / 'result.json'
        self.official = {
            'task_name': 'sample', 'agent_info': {'version': 'source'},
            'exception_info': {'exception_type': 'RuntimeError',
                'exception_message': 'MP-08 / MP-10: task runner dependencies unavailable'},
            'agent_execution': None, 'verifier': None, 'verifier_result': None,
            'agent_result': None,
        }
        self.receipt.write_text(json.dumps(self.official))
        self.identity = {'source_commit': 'source', 'task_ids': ['sample']}
        self.campaign = {**self.identity, 'status': 'blocked', 'finished_at': 10,
            'first_failing_task': 'sample', 'tasks': [{'task_id': 'sample',
                'official_result': str(self.receipt), 'cleanup': {
                    'remaining_containers': [], 'manual_settlement': []}}]}

    def write(self, **changes):
        self.receipt.write_text(json.dumps({**self.official, **changes}))

    def test_archive_before_execution_preserves_receipt(self):
        before = self.receipt.read_bytes()
        result = resume_setup_campaign(self.campaign, self.identity)
        self.assertEqual(result['status'], 'running')
        self.assertEqual(result['tasks'], [])
        self.assertEqual(len(result['setup_attempts']), 1)
        self.assertEqual(self.receipt.read_bytes(), before)
        self.assertNotIn('finished_at', result)

    def test_provider_execution_forbids_retry(self):
        self.write(agent_execution={'started_at': 'observed'})
        with self.assertRaises(ValueError):
            resume_setup_campaign(self.campaign, self.identity)

    def test_verifier_execution_or_result_forbids_retry(self):
        for field in ['verifier', 'verifier_result']:
            self.write(**{field: {'observed': True}})
            with self.assertRaises(ValueError):
                resume_setup_campaign(copy.deepcopy(self.campaign), self.identity)

    def test_provider_measurement_forbids_retry(self):
        self.write(agent_result={'metadata': {'chariox': {'status': 'provider_failed'}}})
        with self.assertRaises(ValueError):
            resume_setup_campaign(self.campaign, self.identity)

    def test_other_failure_forbids_retry(self):
        self.write(exception_info={'exception_type': 'RuntimeError', 'exception_message': 'other'})
        with self.assertRaises(ValueError):
            resume_setup_campaign(self.campaign, self.identity)

    def test_missing_execution_field_forbids_retry(self):
        for field in ['agent_execution', 'verifier', 'verifier_result', 'agent_result']:
            official = {k: v for k, v in self.official.items() if k != field}
            self.receipt.write_text(json.dumps(official))
            with self.assertRaises(ValueError):
                resume_setup_campaign(copy.deepcopy(self.campaign), self.identity)

    def test_unsettled_resources_forbid_retry(self):
        for cleanup in [{'remaining_containers': ['owned'], 'manual_settlement': []},
                        {'remaining_containers': [], 'manual_settlement': [{'retained_volumes': ['unknown']}]},
                        {'remaining_containers': [], 'manual_settlement': [{'removed_containers': ['owned']}] }]:
            campaign = copy.deepcopy(self.campaign)
            campaign['tasks'][-1]['cleanup'] = cleanup
            with self.assertRaises(ValueError):
                resume_setup_campaign(campaign, self.identity)

    def test_identity_or_official_source_mismatch_forbids_retry(self):
        with self.assertRaises(ValueError):
            resume_setup_campaign(copy.deepcopy(self.campaign), {**self.identity, 'source_commit': 'other'})
        self.write(agent_info={'version': 'other'})
        with self.assertRaises(ValueError):
            resume_setup_campaign(self.campaign, self.identity)


if __name__ == '__main__':
    unittest.main()
