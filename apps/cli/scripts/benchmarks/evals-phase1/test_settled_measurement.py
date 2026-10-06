"""MP-08 / MP-10 / MP-11: preserve provider failures as scored attempts, never infrastructure errors."""
import unittest
from contract import settled_measurement, provider_failure_class
class SettledMeasurementTests(unittest.TestCase):
    def test_measured_provider_failure_is_scored_but_quota_or_transport_failure_is_not(self):
        m={'status':'provider_failed','cleanup_complete':True,'tui_usage_visible':True,'session_id':'s','usage':{'session_id':'s','complete':True,'input_tokens':10,'cached_input_tokens':0,'output_tokens':2}}
        self.assertTrue(settled_measurement(m))
        for status in ['quota_exhausted','failed','accounting_projection_failed']:
            self.assertFalse(settled_measurement(dict(m,status=status)))
        self.assertFalse(settled_measurement(dict(m,usage=None)))
        self.assertFalse(settled_measurement(dict(m,cleanup_complete=False)))

    def test_settled_rejection_without_counters_still_reaches_verifier(self):
        m={'status':'provider_failed','provider_turn_settled':True,'cleanup_complete':True,
           'session_id':'s','agent_id':'a','prompt_id':'p','usage':None,'tui_usage_visible':False}
        self.assertTrue(settled_measurement(m))
        for key in ['provider_turn_settled','cleanup_complete','prompt_id']:
            self.assertFalse(settled_measurement(dict(m,**{key:False})))
        self.assertFalse(settled_measurement(dict(m,status='completed')))

    def test_only_new_task_error_entries_classify_auth_or_quota(self):
        entries=[{'id':1,'role':'error','text':'401 Unauthorized'},
                 {'id':2,'role':'user','text':'401 unauthorized quota exceeded'},
                 {'id':3,'role':'error','promptId':'other','text':'401 Unauthorized'},
                 {'id':4,'role':'error','promptId':'p','text':'cyber_policy rejection'}]
        self.assertEqual(provider_failure_class(entries,'p',{1}),'policy_rejected')
        entries.append({'id':5,'role':'error','promptId':'p','text':'HTTP 401 Unauthorized'})
        self.assertEqual(provider_failure_class(entries,'p',{1}),'auth_unauthorized')
        self.assertEqual(provider_failure_class([{'id':6,'role':'error','text':"You've hit your usage limit"}],'p'),'quota_or_rate_limit')
