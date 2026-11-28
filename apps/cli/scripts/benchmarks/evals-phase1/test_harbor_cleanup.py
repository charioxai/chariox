"""MP-11: leftover Harbor containers require full identity and exact ownership."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from harbor_cleanup import settle_job


class HarborCleanupTests(unittest.TestCase):
    def cleanup(self, foreign=False):
        with tempfile.TemporaryDirectory(prefix='chariox-evals-cleanup-') as directory:
            job = Path(directory); (job / 'task__abc123').mkdir()
            (job / 'unrelated__abc123').mkdir()
            project = 'task__abc123__env'; cid = 'a' * 64
            def output(command):
                if command[1] == 'ps':
                    self.assertIn('label=com.docker.compose.project=' + project, command)
                    return cid if '--no-trunc' in command else cid[:12]
                if command[1] == 'inspect':
                    return cid + ' ' + ('foreign-project' if foreign else project)
                return ''
            with patch('harbor_cleanup.output', side_effect=output), \
                 patch('harbor_cleanup.subprocess.run') as mutate:
                if foreign:
                    with self.assertRaisesRegex(RuntimeError, 'identity mismatch'):
                        settle_job(job, 'task')
                    mutate.assert_not_called()
                else:
                    receipt = settle_job(job, 'task')
                    self.assertEqual(receipt[0]['removed_containers'], [cid])
                    self.assertEqual([call.args[0] for call in mutate.call_args_list], [
                        ['docker', 'stop', '--time', '10', cid], ['docker', 'rm', cid]])
                    self.assertEqual(len(receipt), 1)

    def test_owned_leftover_is_stopped_and_removed_using_full_id(self):
        self.cleanup()

    def test_foreign_project_label_is_rejected_before_mutation(self):
        self.cleanup(foreign=True)
