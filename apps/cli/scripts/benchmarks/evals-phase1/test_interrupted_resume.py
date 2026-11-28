"""MP-08 / MP-10 / MP-11: a runner interruption before any new Harbor job resumes without retrying scored work."""
import copy
from pathlib import Path
import tempfile
import unittest

from terminal_campaign import reserve_reached, resume_interrupted_campaign

GIB = 1024 ** 3


class InterruptedResumeTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix='chariox-evals-interrupted-test-')
        self.addCleanup(self.scratch.cleanup)
        self.output = Path(self.scratch.name)
        (self.output / 'jobs' / 'scored-job' / 'trial').mkdir(parents=True)
        self.identity = {'source_commit': 'source', 'task_ids': ['scored', 'next']}
        self.scored = {'task_id': 'scored', 'official_result': str(self.output / 'jobs/scored-job/trial/result.json')}
        self.campaign = {**self.identity, 'status': 'blocked', 'failure_kind': 'RuntimeError', 'finished_at': 10,
                         'tasks': [self.scored]}

    def test_interruption_without_new_job_resumes_and_is_recorded(self):
        result = resume_interrupted_campaign(copy.deepcopy(self.campaign), self.identity, self.output)
        self.assertEqual(result['status'], 'running')
        self.assertEqual(result['tasks'], [self.scored])
        self.assertEqual(result['interruptions'], [{'failure_kind': 'RuntimeError', 'finished_at': 10}])
        self.assertNotIn('failure_kind', result)
        self.assertNotIn('finished_at', result)

    def test_unaccounted_job_or_task_failure_forbids_resume(self):
        (self.output / 'jobs' / 'aborted-job').mkdir()
        with self.assertRaises(ValueError):
            resume_interrupted_campaign(copy.deepcopy(self.campaign), self.identity, self.output)
        (self.output / 'jobs' / 'aborted-job').rmdir()
        for changes in [{'first_failing_task': 'scored'}, {'failure_kind': None}, {'status': 'completed'}]:
            with self.assertRaises(ValueError):
                resume_interrupted_campaign({**copy.deepcopy(self.campaign), **changes}, self.identity, self.output)
        with self.assertRaises(ValueError):
            resume_interrupted_campaign(copy.deepcopy(self.campaign), {**self.identity, 'source_commit': 'other'}, self.output)

    def test_reserve_stops_only_started_harbor_job(self):
        low = {'mem_available_bytes': 7 * GIB, 'root_free_bytes': 80 * GIB}
        self.assertFalse(reserve_reached(low, started=False))
        self.assertTrue(reserve_reached(low, started=True))
        self.assertTrue(reserve_reached({'mem_available_bytes': 20 * GIB, 'root_free_bytes': 14 * GIB}, started=True))
        self.assertFalse(reserve_reached({'mem_available_bytes': 20 * GIB, 'root_free_bytes': 80 * GIB}, started=True))


if __name__ == '__main__':
    unittest.main()
