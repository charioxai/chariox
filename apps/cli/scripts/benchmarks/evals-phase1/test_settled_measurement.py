"""MP-08 / MP-10 / MP-11: preserve provider failures as scored attempts, never infrastructure errors."""
import unittest
from contract import settled_measurement
class SettledMeasurementTests(unittest.TestCase):
    def test_measured_provider_failure_is_scored_but_quota_or_transport_failure_is_not(self):
        m={'status':'provider_failed','cleanup_complete':True,'tui_usage_visible':True,'session_id':'s','usage':{'session_id':'s','complete':True,'input_tokens':10,'cached_input_tokens':0,'output_tokens':2}}
        self.assertTrue(settled_measurement(m))
        for status in ['quota_exhausted','failed','accounting_projection_failed']:
            self.assertFalse(settled_measurement(dict(m,status=status)))
        self.assertFalse(settled_measurement(dict(m,usage=None)))
        self.assertFalse(settled_measurement(dict(m,cleanup_complete=False)))
