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
            prediction = output / 'first.prediction.jsonl'
            def solve(command, **_):
                result.write_text(json.dumps(measurement))
                if existing_prediction:
                    prediction.write_text('existing prediction\n')
                return 0
            argv = ['swe_campaign', '--runtime-root', str(runtime), '--parquet', str(root / 'tasks'),
                    '--profile-path', str(root / 'profile'), '--model', 'model', '--local-protocol', '448',
                    '--workspace-root', str(root / 'workspaces'), '--output-root', str(output)]
            with patch('sys.argv', argv), patch('swe_campaign.preflight'), \
                 patch('swe_campaign.resource_sample', return_value={}), \
                 patch('swe_campaign.load_tasks', return_value={'first': {}, 'second': {}}), \
                 patch('swe_campaign.subprocess.call', side_effect=solve), \
                 patch('swe_campaign.subprocess.run') as checkout:
                (root / 'workspaces' / 'first').mkdir(parents=True)
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

    def test_second_invocation_archives_admission_failure_and_reruns_task(self):
        with tempfile.TemporaryDirectory(prefix='chariox-evals-rerun-') as directory:
            root = Path(directory); runtime = root / 'runtime'; runtime.mkdir()
            (runtime / 'eval-runtime.json').write_text(json.dumps({
                'source_commit': 'a' * 40, 'kernel_sha256': 'b' * 64}))
            output = root / 'output'; workspaces = root / 'workspaces'
            outcomes = [{'status': 'failed', 'cleanup_complete': True, 'usage': None,
                         'first_failing_seam': 'profile_link'},
                        {'status': 'provider_failed', 'cleanup_complete': True, 'provider_turn_settled': True,
                         'session_id': 's', 'agent_id': 'a', 'prompt_id': 'p', 'usage': None}]
            def solve(command, **_):
                result = Path(command[command.index('--result') + 1])
                result.write_text(json.dumps(outcomes.pop(0)))
                result.with_suffix('.evidence').mkdir()
                return 0
            def checkout(command, **_):
                if command[1] == 'init': Path(command[2]).mkdir(parents=True)
            argv = ['swe_campaign', '--runtime-root', str(runtime), '--parquet', str(root / 'tasks'),
                    '--profile-path', str(root / 'profile'), '--model', 'model', '--local-protocol', '448',
                    '--workspace-root', str(workspaces), '--output-root', str(output)]
            with patch('sys.argv', argv), patch('swe_campaign.preflight'), \
                 patch('swe_campaign.resource_sample', return_value={}), \
                 patch('swe_campaign.load_tasks', return_value={'task': {'repo': 'r', 'base_commit': 'c'}}), \
                 patch('swe_campaign.subprocess.run', side_effect=checkout), \
                 patch('swe_campaign.subprocess.call', side_effect=solve):
                self.assertEqual(swe_campaign.main(), 2)
                (workspaces / 'task' / 'edit').write_text('unscored edit')
                self.assertEqual(swe_campaign.main(), 0)
            self.assertEqual(outcomes, [])
            archived = list((output / 'unsettled-attempts').rglob('task.json'))
            self.assertEqual([json.loads(p.read_text())['status'] for p in archived], ['failed'])
            self.assertTrue((archived[0].parent / 'task.evidence').is_dir())
            self.assertEqual(len(list((workspaces / 'unsettled-attempts').rglob('edit'))), 1)
            self.assertFalse((workspaces / 'task' / 'edit').exists())
            self.assertEqual(json.loads((output / 'predictions.jsonl').read_text())['model_patch'], '')

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
