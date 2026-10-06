"""MP-08 / MP-10 / MP-11: preserve official successes while retrying only quota admission."""
import copy,json,tempfile,unittest
from pathlib import Path
from terminal_campaign import resume_quota_campaign
class TerminalResumeTests(unittest.TestCase):
    def test_quota_attempt_preserved_without_replaying_scored_task(self):
        with tempfile.TemporaryDirectory() as directory:
            result=Path(directory)/'result.json'
            result.write_text(json.dumps({'agent_result':{'metadata':{'chariox':{'status':'quota_exhausted','cleanup_complete':True}}}}))
            scored={'task_id':'first','official_result':'unchanged'}
            quota={'task_id':'second','official_result':str(result),'cleanup':{'remaining_containers':[]}}
            campaign={'model':'gpt-6.1-sol','status':'blocked','tasks':[scored,quota],'first_failing_task':'second'}
            new=resume_quota_campaign(campaign,{'model':'gpt-6.1-sol'})
            self.assertEqual(new['tasks'],[scored]);self.assertEqual(new['quota_attempts'],[quota]);self.assertTrue(result.exists())
            self.assertEqual(new['status'],'running')
    def test_changed_identity_or_harness_error_cannot_be_silently_retried(self):
        with self.assertRaises(ValueError):resume_quota_campaign({'model':'other'},{'model':'gpt-6.1-sol'})
        with self.assertRaises(ValueError):resume_quota_campaign({'status':'completed','tasks':[]},{})
