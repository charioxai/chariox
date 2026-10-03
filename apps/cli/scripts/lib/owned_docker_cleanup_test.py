"""MP-08 / MP-10: exercise delayed auto-removal, ownership and bounded failure."""
import unittest
from unittest.mock import patch
from owned_docker_cleanup import stop_owned_container

class CleanupTests(unittest.TestCase):
    def run_cleanup(self, replies, **kwargs):
        remaining = iter(replies)
        return stop_owned_container('owned', 'image', 'owner', lambda *_: next(remaining), ['docker'], **kwargs)

    @patch('owned_docker_cleanup.time.sleep')
    @patch('owned_docker_cleanup.subprocess.run')
    def test_waits_for_docker_auto_removal(self, stop, sleep):
        result = self.run_cleanup([(0, 'image'), (0, 'owner'), (0, 'id'), (0, 'id'), (1, '')])
        self.assertTrue(result['absent'])
        self.assertEqual(sleep.call_count, 2)
        stop.assert_called_once()

    @patch('owned_docker_cleanup.subprocess.run')
    def test_rejects_unowned_containers_before_stop(self, stop):
        for replies in [[(0, 'other-image')], [(0, 'image'), (0, 'other-owner')]]:
            with self.assertRaises(ValueError): self.run_cleanup(replies)
        stop.assert_not_called()

    @patch('owned_docker_cleanup.subprocess.run')
    def test_removal_deadline_is_a_failure(self, stop):
        result = self.run_cleanup([(0, 'image'), (0, 'owner'), (0, 'id')], timeout=0)
        self.assertFalse(result['absent'])
        self.assertEqual(result['error'], 'auto_removal_timeout')

    @patch('owned_docker_cleanup.subprocess.run')
    def test_already_absent_needs_no_stop(self, stop):
        self.assertTrue(self.run_cleanup([(1, '')])['alreadyAbsent'])
        stop.assert_not_called()

if __name__ == '__main__': unittest.main()
