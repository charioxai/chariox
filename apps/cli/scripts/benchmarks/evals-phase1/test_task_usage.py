"""MP-08 / MP-10: a leased benchmark task excludes old and unrelated turns."""
import unittest
from contract import task_usage

class TaskUsageTests(unittest.TestCase):
    def test_old_task_and_unrelated_active_agent_do_not_change_measurement(self):
        def turn(agent,prompt,completed,amount):
            return {'agent_id':agent,'prompt_id':prompt,'provider_run_id':'run-'+prompt,'completed':completed,'usage':{
                'input_tokens':amount,'cached_input_tokens':0,'output_tokens':1,'reasoning_tokens':0},'api_equivalent_nanodollars':None}
        report={'session_id':'s','turns':[turn('a','old',True,1000),turn('a','current',True,20),turn('other','active',False,999)]}
        result=task_usage(report,'s','a','current')
        self.assertEqual(result['input_tokens'],20)
        self.assertEqual(result['provider_run_ids'],['run-current'])
        self.assertTrue(result['complete'])

    def test_missing_or_incomplete_selected_turn_is_unavailable(self):
        for turns in [[],[{'agent_id':'a','prompt_id':'current','provider_run_id':'r','completed':False,'usage':None}]]:
            self.assertIsNone(task_usage({'session_id':'s','turns':turns},'s','a','current'))

    def test_foreign_session_and_ambiguous_duplicate_binding_rejected(self):
        with self.assertRaises(ValueError):task_usage({'session_id':'foreign'},'s','a','p')

    def test_descendants_require_explicit_agent_and_prompt_bindings(self):
        def turn(a,p,n):
            return {'agent_id':a,'prompt_id':p,'provider_run_id':p,'completed':True,'usage':{'input_tokens':n,'cached_input_tokens':0,'output_tokens':1,'cache_write':2},'api_equivalent_nanodollars':None}
        report={'session_id':'s','turns':[turn('a','p',10),turn('child','old',500),turn('child','task-child',20)]}
        self.assertEqual(task_usage(report,'s','a','p')['input_tokens'],10)
        result=task_usage(report,'s','a','p',[('child','task-child')])
        self.assertEqual(result['input_tokens'],30)
        self.assertEqual(result['cache_write_tokens'],4)
        self.assertIsNone(task_usage(report,'s','a','p',[('child','missing')]))


class SessionProjectionTests(unittest.TestCase):
    def test_session_projection_is_independent_of_task_totals(self):
        from contract import session_usage_visible
        report={'total':{'turns':2,'usage':{'input_tokens':1020,'cached_input_tokens':None,'output_tokens':24,'reasoning_tokens':None}}}
        self.assertTrue(session_usage_visible(report,'Session: 2 turns; input 1020; cached unavailable; output 24; reasoning unavailable;'))
        self.assertFalse(session_usage_visible(report,'Session: 2 turns; input 20; cached unavailable; output 24; reasoning unavailable;'))
