"""MP-08 / MP-10 / MP-11: one task runner, local or coordinator-granted lease."""
import unittest
from contract import admit_placement, leased_target


class PlacementTests(unittest.TestCase):
    def request(self):
        return {'placement': 'leased', 'home_kernel_url': 'ws://127.0.0.1:50000',
                'home_session_ref': 'session', 'home_agent_id': 'agent',
                'worker_kernel_id': 'worker', 'provider': 'claude', 'model': 'claude-sonnet-4-6'}

    def snapshot(self):
        return {'session': {'id': 'session', 'agents': [{'id': 'agent', 'provider': 'claude',
                'model': 'claude-sonnet-4-6', 'remoteExecution': {'workerKernelId': 'worker'}}]}}

    def test_lease_needs_no_builder_profile_or_login(self):
        self.assertEqual(admit_placement(self.request()), 'leased')
        with self.assertRaises(ValueError):
            admit_placement(dict(self.request(), profile_path='/a/builder/profile'))

    def test_missing_grant_does_not_fall_back_to_local(self):
        request = self.request(); request.pop('worker_kernel_id')
        with self.assertRaises(ValueError):
            admit_placement(request)

    def test_local_requires_explicit_product_link(self):
        with self.assertRaises(ValueError):
            admit_placement({'placement': 'local'})
        self.assertEqual(admit_placement({'profile_path': '/approved/product-profile'}), 'local')

    def test_foreign_worker_or_agent_is_rejected(self):
        for field in ['worker_kernel_id', 'home_agent_id']:
            with self.assertRaises(ValueError):
                leased_target(self.snapshot(), dict(self.request(), **{field: 'foreign'}))

    def test_wrong_provider_or_model_is_rejected(self):
        for field in ['provider', 'model']:
            with self.assertRaises(ValueError):
                leased_target(self.snapshot(), dict(self.request(), **{field: 'other'}))

    def test_foreign_home_session_is_rejected(self):
        with self.assertRaises(ValueError):
            leased_target(self.snapshot(), dict(self.request(), home_session_ref='foreign'))

    def test_matching_binding_selects_only_the_task_agent(self):
        snapshot = self.snapshot()
        snapshot['session']['agents'].append({'id': 'unrelated'})
        self.assertEqual(leased_target(snapshot, self.request())['id'], 'agent')


if __name__ == '__main__':
    unittest.main()
