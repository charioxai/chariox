"""MP-08 / MP-11: owned desktop supervisor signal boundaries."""
import importlib.util
from pathlib import Path
import signal
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('session', Path(__file__).with_name('linux-desktop-session.py'))
session = importlib.util.module_from_spec(spec)
spec.loader.exec_module(session)


class SessionTests(unittest.TestCase):
    def test_invalid_targets_never_reach_kill(self):
        with patch.object(session.os, 'kill') as kill:
            for pid in [0, 1, -1, None, float('nan'), float('inf'), 1.1, 2147483648]:
                with self.assertRaises(ValueError):
                    session.signal_child({'pid': pid}, signal.SIGTERM)
            kill.assert_not_called()

    def test_stale_and_foreign_children_never_reach_kill(self):
        item = {'pid': 123, 'started': '456'}
        with patch.object(session.os, 'getpid', return_value=100), patch.object(session.os, 'kill') as kill:
            for current in [None, {'parent': 1, 'started': '456'}, {'parent': 100, 'started': '457'}]:
                with patch.object(session, 'identity', return_value=current):
                    self.assertFalse(session.signal_child(item, signal.SIGTERM))
            kill.assert_not_called()

    def test_exact_direct_child_can_be_signalled(self):
        with patch.object(session.os, 'getpid', return_value=100), patch.object(session, 'identity', return_value={'parent': 100, 'started': '456'}), patch.object(session.os, 'kill') as kill:
            self.assertTrue(session.signal_child({'pid': 123, 'started': '456'}, signal.SIGTERM))
            kill.assert_called_once_with(123, signal.SIGTERM)


if __name__ == '__main__':
    unittest.main()
