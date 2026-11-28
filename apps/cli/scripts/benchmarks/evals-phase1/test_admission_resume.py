"""MP-08 / MP-10 / MP-11: retry only a clean pre-prompt admission failure."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from terminal_campaign import resume_admission_campaign


class AdmissionResumeTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix='chariox-evals-admission-test-')
        self.addCleanup(self.scratch.cleanup)
        self.receipt = Path(self.scratch.name) / 'result.json'
        self.measurement = {'status': 'failed', 'first_failing_seam': 'profile_link', 'usage': None,
                            'failure_kind': 'CalledProcessError', 'cleanup_complete': True}
        self.official = {
            'task_name': 'sample', 'agent_info': {'version': 'source'},
            'exception_info': {'exception_type': 'RuntimeError',
                'exception_message': 'MP-08 / MP-10: Chariox task did not settle; retain failed task in campaign ledger'},
            'agent_execution': {'started_at': 'observed'}, 'verifier': None, 'verifier_result': None,
            'agent_result': {'metadata': {'chariox': self.measurement}},
        }
        self.receipt.write_text(json.dumps(self.official))
        self.identity = {'source_commit': 'source', 'task_ids': ['scored', 'sample']}
        self.scored = {'task_id': 'scored', 'official_result': 'unchanged'}
        self.campaign = {**self.identity, 'status': 'blocked', 'finished_at': 10,
            'first_failing_task': 'sample', 'tasks': [self.scored, {'task_id': 'sample',
                'official_result': str(self.receipt), 'cleanup': {
                    'remaining_containers': [], 'manual_settlement': []}}]}

    def measured(self, **changes):
        self.receipt.write_text(json.dumps({**self.official, 'agent_result': {
            'metadata': {'chariox': {**self.measurement, **changes}}}}))

    def rejects(self, campaign=None, identity=None):
        with self.assertRaises(ValueError):
            resume_admission_campaign(campaign or copy.deepcopy(self.campaign), identity or self.identity)

    def test_pre_prompt_failure_archived_and_scored_tasks_kept(self):
        before = self.receipt.read_bytes()
        result = resume_admission_campaign(self.campaign, self.identity)
        self.assertEqual(result['status'], 'running')
        self.assertEqual(result['tasks'], [self.scored])
        self.assertEqual([a['task_id'] for a in result['admission_attempts']], ['sample'])
        self.assertEqual(self.receipt.read_bytes(), before)
        self.assertNotIn('finished_at', result)
        self.assertNotIn('first_failing_task', result)

    def test_prompt_or_later_seam_forbids_retry(self):
        for changes in [{'first_failing_seam': 'prompt_submit'}, {'first_failing_seam': 'provider_turn'},
                        {'prompt_id': 'prompt'}, {'status': 'provider_failed'}, {'usage': {'input_tokens': 1}},
                        {'cleanup_complete': False}]:
            self.measured(**changes)
            self.rejects()

    def test_verifier_or_other_failure_forbids_retry(self):
        for changes in [{'verifier': {'observed': True}}, {'verifier_result': {'rewards': {'reward': 0}}},
                        {'exception_info': {'exception_type': 'RuntimeError', 'exception_message': 'other'}},
                        {'agent_info': {'version': 'other'}}, {'task_name': 'other'}]:
            self.receipt.write_text(json.dumps({**self.official, **changes}))
            self.rejects()

    def test_identity_status_or_unsettled_resources_forbid_retry(self):
        self.rejects(identity={**self.identity, 'source_commit': 'other'})
        self.rejects(campaign={**copy.deepcopy(self.campaign), 'status': 'completed'})
        for cleanup in [{'remaining_containers': ['owned'], 'manual_settlement': []},
                        {'remaining_containers': [], 'manual_settlement': [{'retained_volumes': ['unknown']}]}]:
            campaign = copy.deepcopy(self.campaign)
            campaign['tasks'][-1]['cleanup'] = cleanup
            self.rejects(campaign=campaign)


if __name__ == '__main__':
    unittest.main()
