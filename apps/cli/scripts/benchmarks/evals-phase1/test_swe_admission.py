"""MP-08 / MP-10 / MP-11: admission blockers never become SWE predictions."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import swe_campaign


class SweAdmissionTests(unittest.TestCase):
    def campaign(self, measurement, existing_prediction=False):
        with tempfile.TemporaryDirectory(prefix='chariox-evals-admission-') as directory:
            root = Path(directory)
            runtime = root / 'runtime'; runtime.mkdir()
            (runtime / 'eval-runtime.json').write_text(json.dumps({
                'source_commit': 'a' * 40, 'kernel_sha256': 'b' * 64}))
            output = root / 'output'; output.mkdir()
            result = output / 'first.json'
            result.write_text(json.dumps(measurement))
            prediction = output / 'first.prediction.jsonl'
            if existing_prediction:
                prediction.write_text('existing prediction\n')
            argv = ['swe_campaign', '--runtime-root', str(runtime), '--parquet', str(root / 'tasks'),
                    '--profile-path', str(root / 'profile'), '--model', 'model', '--local-protocol', '448',
                    '--workspace-root', str(root / 'workspaces'), '--output-root', str(output)]
            with patch('sys.argv', argv), patch('swe_campaign.preflight'), \
                 patch('swe_campaign.resource_sample', return_value={}), \
                 patch('swe_campaign.load_tasks', return_value={'first': {}, 'second': {}}), \
                 patch('swe_campaign.subprocess.run') as checkout:
                # Reaching the second task reproduces the old campaign advancement.
                checkout.side_effect = AssertionError('admission failure advanced to next task')
                code = swe_campaign.main()
            self.assertEqual(code, 2)
            self.assertEqual(json.loads(result.read_text()), measurement)
            self.assertEqual(prediction.exists(), existing_prediction)
            self.assertFalse((output / 'predictions.jsonl').exists())
            self.assertFalse((output / 'second.json').exists())

    def test_pre_prompt_auth_refresh_failure_stops_unscored(self):
        self.campaign({'status': 'failed', 'cleanup_complete': True, 'usage': None,
                       'session_id': None, 'agent_id': None, 'prompt_id': None,
                       'first_failing_seam': 'profile_refresh: 401 Unauthorized'})

    def test_auth_quota_transport_and_unsettled_failures_stop_even_with_prediction(self):
        for status in ['provider_auth_failed', 'provider_quota_unconfirmed', 'failed',
                       'accounting_projection_failed', 'provider_failed', 'completed']:
            with self.subTest(status=status):
                self.campaign({'status': status, 'cleanup_complete': True, 'usage': None}, True)

    def test_admitted_settled_rejection_keeps_empty_prediction(self):
        # A settled provider rejection is a solver outcome, unlike admission failure.
        with tempfile.TemporaryDirectory(prefix='chariox-evals-settled-') as directory:
            root = Path(directory); runtime = root / 'runtime'; runtime.mkdir()
            (runtime / 'eval-runtime.json').write_text(json.dumps({
                'source_commit': 'a' * 40, 'kernel_sha256': 'b' * 64}))
            output = root / 'output'; output.mkdir()
            (output / 'task.json').write_text(json.dumps({
                'status': 'provider_failed', 'cleanup_complete': True, 'provider_turn_settled': True,
                'session_id': 's', 'agent_id': 'a', 'prompt_id': 'p', 'usage': None}))
            argv = ['swe_campaign', '--runtime-root', str(runtime), '--parquet', str(root / 'tasks'),
                    '--profile-path', str(root / 'profile'), '--model', 'model', '--local-protocol', '448',
                    '--workspace-root', str(root / 'workspaces'), '--output-root', str(output)]
            with patch('sys.argv', argv), patch('swe_campaign.preflight'), \
                 patch('swe_campaign.resource_sample', return_value={}), \
                 patch('swe_campaign.load_tasks', return_value={'task': {}}):
                self.assertEqual(swe_campaign.main(), 0)
            self.assertEqual(json.loads((output / 'predictions.jsonl').read_text())['model_patch'], '')
