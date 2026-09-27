#!/usr/bin/env python3
import copy
from datetime import datetime, timezone
import hashlib
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location('cleanup_guard', Path(__file__).with_name('image-builder-cleanup.py'))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def fixture():
    public_key = 'ssh-ed25519 AAAA-test-public-only'
    receipt = {
        'format': guard.FORMAT, 'runId': 'chariox-image-fixture', 'sourceCommit': 'a' * 40,
        'expiresAt': '2026-09-27T20:00:00Z',
        'server': {'id': 11, 'name': 'chariox-image-fixture', 'created': '2026-09-27T18:00:00Z',
                   'ipv4': '192.0.2.5', 'type': 'cpx22', 'location': 'fsn1', 'imageId': 387894169},
        'firewall': {'id': 22, 'name': 'chariox-image-fixture-ssh'},
        'sshKey': {'id': 33, 'name': 'chariox-image-fixture-key',
                   'publicKeySha256': hashlib.sha256(public_key.encode()).hexdigest()},
    }
    server = {**receipt['server'], 'labels': guard.labels(receipt), 'server_type': {'name': 'cpx22'},
              'location': {'name': 'fsn1'}, 'datacenter': {'location': {'name': 'fsn1'}}, 'image': {'id': 387894169}, 'volumes': [],
              'rescue_enabled': False, 'backup_window': None, 'protection': {'delete': False},
              'public_net': {'ipv4': {'ip': '192.0.2.5'}, 'firewalls': [{'id': 22}]}}
    firewall = {**receipt['firewall'], 'labels': guard.labels(receipt),
                'rules': [{'direction': 'in', 'protocol': 'tcp', 'port': '22', 'source_ips': ['51.38.82.47/32']}],
                'applied_to': [{'type': 'server', 'server': {'id': 11}}]}
    key = {**receipt['sshKey'], 'labels': guard.labels(receipt), 'public_key': public_key + ' temporary'}
    return receipt, FakeProvider(server, firewall, key)


class FakeProvider:
    def __init__(self, server, firewall, key):
        self.resources = {'servers/11': {'server': server}, 'firewalls/22': {'firewall': firewall},
                          'ssh_keys/33': {'ssh_key': key}}
        self.calls = []
        self.action = {'id': 44, 'command': 'delete_server', 'resources': [{'id': 11, 'type': 'server'}],
                       'status': 'success', 'error': None}
        self.retain_server = False
        self.after_server_delete = lambda: None
        self.fail_delete = None

    def request(self, route, method='GET', allow_absent=False):
        self.calls.append((method, route))
        if method == 'DELETE':
            if self.fail_delete == route:
                raise ValueError('uncertain provider delete')
            if route == 'servers/11':
                if not self.retain_server:
                    del self.resources[route]
                    self.resources['firewalls/22']['firewall']['applied_to'] = []
                self.after_server_delete()
                return {'action': {'id': 44}}
            del self.resources[route]
            return {}
        if route == 'actions/44':
            return {'action': copy.deepcopy(self.action)}
        if route in self.resources:
            return copy.deepcopy(self.resources[route])
        if allow_absent:
            return None
        raise AssertionError('unexpected API request ' + route)

    def deletes(self):
        return [route for method, route in self.calls if method == 'DELETE']


class CleanupTests(unittest.TestCase):
    def test_timestamp_remains_compatible_with_creator_timer(self):
        for suffix in ['Z', '+00:00']:
            self.assertEqual(datetime.fromtimestamp(guard.timestamp('2026-09-27T20:00:00' + suffix), timezone.utc),
                             datetime(2026, 9, 27, 20, tzinfo=timezone.utc))

    def test_equivalent_utc_timestamp_formatting_preserves_identity(self):
        for receipt_suffix, provider_suffix in [('Z', '+00:00'), ('+00:00', 'Z')]:
            with self.subTest(receipt_suffix=receipt_suffix):
                receipt, api = fixture()
                receipt['server']['created'] = '2026-09-27T18:00:00.123456789' + receipt_suffix
                receipt['expiresAt'] = '2026-09-27T20:00:00' + provider_suffix
                api.resources['servers/11']['server']['created'] = '2026-09-27T18:00:00.1234567890' + provider_suffix
                guard.validate_receipt(receipt)
                self.assertTrue(guard.cleanup(api, receipt, apply=True)['cleaned'])

    def test_changed_creation_instant_rejected_without_precision_loss(self):
        for changed in ['2026-09-27T18:00:01+00:00', '2026-09-27T18:00:00.123456788+00:00']:
            with self.subTest(changed=changed):
                receipt, api = fixture()
                receipt['server']['created'] = '2026-09-27T18:00:00.123456789Z'
                api.resources['servers/11']['server']['created'] = changed
                with self.assertRaises(ValueError):
                    guard.cleanup(api, receipt, apply=True)
                self.assertEqual(api.deletes(), [])

    def test_timestamp_rejects_naive_non_utc_and_invalid_dates(self):
        for value in [None, '2026-09-27T18:00:00', '2026-09-27T18:00:00+01:00',
                      '2026-09-27T18:00:00-00:00', '2026-09-27T18:00:00+00:01',
                      '2026-02-30T18:00:00Z']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                guard.timestamp(value)

    def test_current_location_shape_and_conflicting_legacy_location(self):
        receipt, api = fixture()
        server = api.resources['servers/11']['server']
        server['datacenter'] = None
        guard.validate_server(server, receipt)
        server['datacenter'] = {'location': {'name': 'nbg1'}}
        with self.assertRaises(ValueError):
            guard.validate_server(server, receipt)
        server['datacenter'] = None
        server['location'] = None
        with self.assertRaises(ValueError):
            guard.validate_server(server, receipt)

    def test_check_never_mutates(self):
        receipt, api = fixture()
        self.assertEqual(guard.validate_receipt(receipt), receipt)
        self.assertTrue(guard.cleanup(api, receipt)['serverPresent'])
        self.assertEqual(api.deletes(), [])

    def test_cleanup_exact_owned_assets_and_confirm_absence(self):
        receipt, api = fixture()
        self.assertTrue(guard.cleanup(api, receipt, apply=True)['cleaned'])
        self.assertEqual(api.deletes(), ['servers/11', 'firewalls/22', 'ssh_keys/33'])
        self.assertEqual(api.resources, {})
        self.assertFalse(any('images' in route for _, route in api.calls))

    def test_retry_after_success_is_read_only(self):
        receipt, api = fixture()
        guard.cleanup(api, receipt, apply=True)
        api.calls.clear()
        self.assertTrue(guard.cleanup(api, receipt, apply=True)['cleaned'])
        self.assertEqual(api.deletes(), [])

    def test_cleanup_remaining_owned_assets_after_server_absent(self):
        receipt, api = fixture()
        del api.resources['servers/11']
        api.resources['firewalls/22']['firewall']['applied_to'] = []
        guard.cleanup(api, receipt, apply=True)
        self.assertEqual(api.deletes(), ['firewalls/22', 'ssh_keys/33'])

    def test_foreign_or_changed_identity_fails_before_delete(self):
        mutations = [
            ('servers/11', 'server', lambda x: x.update(id=164975036)),
            ('servers/11', 'server', lambda x: x['labels'].update({'chariox.dev/managed': 'true'})),
            ('servers/11', 'server', lambda x: x.update(created='2026-09-27T18:01:00Z')),
            ('servers/11', 'server', lambda x: x.update(volumes=[123])),
            ('servers/11', 'server', lambda x: x['public_net']['ipv4'].update(ip='2.28.23.91')),
            ('servers/11', 'server', lambda x: x['image'].update(id=428409641)),
            ('servers/11', 'server', lambda x: x['protection'].update(delete=True)),
            ('firewalls/22', 'firewall', lambda x: x['rules'][0].update(source_ips=['0.0.0.0/0'])),
            ('firewalls/22', 'firewall', lambda x: x['applied_to'].append({'type': 'server', 'server': {'id': 164975036}})),
            ('firewalls/22', 'firewall', lambda x: x['applied_to'].append({'type': 'label_selector', 'label_selector': {'selector': 'all'}})),
            ('ssh_keys/33', 'ssh_key', lambda x: x.update(public_key='ssh-ed25519 DIFFERENT')),
            ('ssh_keys/33', 'ssh_key', lambda x: x.update(labels={})),
        ]
        for route, key, mutate in mutations:
            with self.subTest(route=route, mutation=mutate):
                receipt, api = fixture()
                mutate(api.resources[route][key])
                with self.assertRaises(ValueError):
                    guard.cleanup(api, receipt, apply=True)
                self.assertEqual(api.deletes(), [])

    def test_failed_or_unsettled_server_delete_preserves_access(self):
        for change in [lambda api: api.action.update(status='error'),
                       lambda api: api.action.update(resources=[{'id': 164975036, 'type': 'server'}]),
                       lambda api: setattr(api, 'retain_server', True),
                       lambda api: api.action.update(status='running'),
                       lambda api: setattr(api, 'fail_delete', 'servers/11')]:
            receipt, api = fixture()
            change(api)
            with self.assertRaises(ValueError):
                guard.cleanup(api, receipt, apply=True, sleep=lambda _: None)
            self.assertEqual(api.deletes(), ['servers/11'])
            self.assertIn('firewalls/22', api.resources)
            self.assertIn('ssh_keys/33', api.resources)

    def test_recheck_prevents_deleting_reassigned_firewall(self):
        receipt, api = fixture()
        api.after_server_delete = lambda: api.resources['firewalls/22']['firewall'].update(
            applied_to=[{'type': 'server', 'server': {'id': 164975036}}])
        with self.assertRaises(ValueError):
            guard.cleanup(api, receipt, apply=True)
        self.assertEqual(api.deletes(), ['servers/11'])

    def test_no_blind_write_retry(self):
        receipt, api = fixture()
        api.fail_delete = 'firewalls/22'
        with self.assertRaises(ValueError):
            guard.cleanup(api, receipt, apply=True)
        self.assertEqual(api.deletes(), ['servers/11', 'firewalls/22'])

    def test_receipt_rejects_loose_scope_and_unbounded_deadline(self):
        for change in [lambda r: r.update(expiresAt='2026-09-28T18:00:00Z'),
                       lambda r: r.update(expiresAt='2026-09-27T17:00:00Z'),
                       lambda r: r.update(runId='../worker'),
                       lambda r: r['server'].update(type='cpx62'),
                       lambda r: r['server'].update(id=True),
                       lambda r: r.update(snapshotId=999)]:
            receipt, _ = fixture()
            change(receipt)
            with self.assertRaises(ValueError):
                guard.validate_receipt(receipt)

    def test_duplicate_json_and_redirect_rejected(self):
        with self.assertRaises(ValueError):
            guard.unique_object([('id', 1), ('id', 2)])
        with self.assertRaises(ValueError):
            guard.NoRedirect().redirect_request(None, None, 302, '', {}, 'https://other.invalid')


if __name__ == '__main__':
    unittest.main()
