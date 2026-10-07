"""MP-08 / MP-10: missing tasks and changed identities cannot become a full score."""
import json
from pathlib import Path
import tempfile
import unittest
from terminal_report import report

class ReportTests(unittest.TestCase):
    def setup_campaign(self, root, count=10):
        lock=json.loads(Path(__file__).with_name('inputs.lock.json').read_text())
        ids=lock['terminal_bench_2']['task_ids'][:10]
        campaign={'phase':'smoke','task_ids':ids,'harness_revision':lock['harbor']['revision'],
                  'task_revision':lock['terminal_bench_2']['revision'],'source_commit':'a'*40,'kernel_sha256':'b'*64,
                  'model':'gpt-6.1-sol','status':'completed','started_at':0,'finished_at':10,'tasks':[]}
        for task in ids[:count]:
            path=root/(task+'.json')
            path.write_text(json.dumps({'task_name':task,'agent_info':{'version':'a'*40},'agent_result':{'metadata':{'chariox':{
                'source_commit':'a'*40,'kernel_sha256':'b'*64,'session_id':'s','status':'completed','cleanup_complete':True,
                'usage':{'session_id':'s','complete':True,'input_tokens':10,'cached_input_tokens':5,'output_tokens':2,'reasoning_tokens':1}}}},
                'verifier_result':{'rewards':{'reward':1 if task==ids[0] else 0}},'exception_info':None}))
            campaign['tasks'].append({'task_id':task,'official_result':str(path),'wall_time_seconds':1,'cleanup':{'remaining_containers':[]}})
        (root/'campaign.json').write_text(json.dumps(campaign))
        return campaign

    def test_unresolved_tasks_remain_in_denominator_and_reasoning_is_not_added(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);self.setup_campaign(root)
            s=report(root,root/'report')
            self.assertEqual(s['accuracy_percent'],10)
            self.assertEqual(s['tokens']['output_tokens'],20)
            self.assertEqual(s['tokens']['reasoning_tokens'],10)
            self.assertIsNotNone(s['proxy_cost'])

    def test_incomplete_campaign_has_no_full_accuracy_or_fake_zero_tokens(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);self.setup_campaign(root,count=1)
            s=report(root,root/'report')
            self.assertFalse(s['complete']);self.assertIsNone(s['accuracy_percent'])
            self.assertIsNone(s['tokens']['input_tokens']);self.assertIsNone(s['proxy_cost'])
            self.assertEqual(s['denominator'],10)

    def test_official_runtime_identity_drift_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);campaign=self.setup_campaign(root)
            path=Path(campaign['tasks'][0]['official_result']);record=json.loads(path.read_text())
            record['agent_info']['version']='c'*40;path.write_text(json.dumps(record))
            with self.assertRaises(ValueError):report(root,root/'report')

    def test_unmeasured_rejection_preserves_known_subtotal_and_unknown_total(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);campaign=self.setup_campaign(root)
            path=Path(campaign['tasks'][1]['official_result']);record=json.loads(path.read_text())
            record['agent_result']['metadata']['chariox'].update(status='provider_failed',usage=None)
            path.write_text(json.dumps(record))
            s=report(root,root/'report')
            self.assertTrue(s['complete']);self.assertIsNone(s['tokens']['input_tokens'])
            self.assertEqual(s['known_token_subtotal']['input_tokens'],90)
            self.assertEqual(s['unknown_token_tasks']['input_tokens'],1)
            self.assertEqual(s['provider_failures'],1)
            self.assertIsNone(s['proxy_cost'])
            self.assertIsNotNone(s['known_proxy_subtotal'])

    def test_pre_prompt_auth_failure_is_not_solver_wall_or_a_scored_task(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);campaign=self.setup_campaign(root,count=2)
            for index,item in enumerate(campaign['tasks']):
                path=Path(item['official_result']);record=json.loads(path.read_text())
                metadata=record['agent_result']['metadata']['chariox']
                metadata['wall_time_seconds']=7 if index==0 else 23
                if index==1:
                    metadata.update(status='failed',usage=None,first_failing_seam='profile_link')
                    record['verifier_result']=None
                    record['exception_info']={'exception_type':'RuntimeError'}
                path.write_text(json.dumps(record))
            s=report(root,root/'report')
            self.assertEqual(s['scored'],1)
            self.assertEqual(s['solver_wall_time_seconds'],7)
            self.assertIsNone(s['accuracy_percent'])

    def test_job_wall_counts_archived_attempts_but_not_idle_resume_pause(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);campaign=self.setup_campaign(root)
            campaign.update(finished_at=10_000,admission_attempts=[{'task_id':campaign['task_ids'][-1],'wall_time_seconds':4}],
                            setup_attempts=[{'task_id':campaign['task_ids'][0],'wall_time_seconds':3}])
            (root/'campaign.json').write_text(json.dumps(campaign))
            s=report(root,root/'report')
            self.assertEqual(s['wall_time_seconds'],10_000)
            self.assertEqual(s['harbor_job_wall_time_seconds'],17)
            self.assertEqual(s['archived_attempts'],{'admission':1,'quota':0,'setup':1})
