"""MP-11: no system/undefined PID can reach the harness signal operation."""
import unittest
from types import SimpleNamespace
from signal_guard import guarded_signal

class SignalTests(unittest.TestCase):
    def test_invalid_pids_never_reach_sender(self):
        sent=[]; guarded=guarded_signal(lambda p:sent.append(p.pid))
        for pid in [0,1,-1,-100,None,float('nan'),True]:
            with self.assertRaises(ValueError): guarded(SimpleNamespace(pid=pid))
        self.assertEqual(sent,[])
        guarded(SimpleNamespace(pid=1234))
        self.assertEqual(sent,[1234])
