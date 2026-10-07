"""MP-08 / MP-10 / MP-11: archive quota admission only, never rerun scored work."""
import json
from pathlib import Path
import tempfile
import unittest
from swe_campaign import archive_unsettled_attempt

class QuotaResumeTests(unittest.TestCase):
    def test_exhausted_clean_attempt_is_preserved_and_new_attempt_can_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); result=root/'task.json'
            result.write_text(json.dumps({'status':'quota_exhausted','cleanup_complete':True}))
            evidence=root/'task.evidence';evidence.mkdir();(evidence/'screen.txt').write_text('quota')
            self.assertTrue(archive_unsettled_attempt(result,root/'task.prediction.jsonl'))
            self.assertFalse(result.exists());self.assertFalse(evidence.exists())
            self.assertEqual(len(list((root/'quota-attempts').rglob('task.json'))),1)
            self.assertFalse(archive_unsettled_attempt(result,root/'task.prediction.jsonl'))

    def test_completed_or_unclean_attempt_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);result=root/'task.json';prediction=root/'task.prediction.jsonl'
            result.write_text(json.dumps({'status':'provider_failed','cleanup_complete':True,'provider_turn_settled':True,'session_id':'s','agent_id':'a','prompt_id':'p','usage':None}))
            self.assertFalse(archive_unsettled_attempt(result,prediction));self.assertTrue(result.exists())
            result.write_text(json.dumps({'status':'quota_exhausted','cleanup_complete':False}))
            with self.assertRaises(RuntimeError):archive_unsettled_attempt(result,prediction)
            self.assertTrue(result.exists())
