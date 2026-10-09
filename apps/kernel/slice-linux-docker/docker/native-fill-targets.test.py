"""MP-08/MP-11: native fill identity, value retirement and private storage."""
import hashlib
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
spec = importlib.util.spec_from_file_location('fill_targets', Path(__file__).with_name('native-fill-targets.py'))
fill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fill)

class FillTests(unittest.TestCase):
    def test_mp11_plain_replacement_clear_and_password_toggle(self):
        value = 'public-test-canary'
        target = {'pid': 200, 'started': 'one', 'path': '/entry', 'value_hash': hashlib.sha256(value.encode()).hexdigest(), 'length': len(value)}
        node = SimpleNamespace(getRole=lambda: 0, queryText=lambda: SimpleNamespace(characterCount=len(value)))
        with patch.dict(sys.modules, pyatspi=SimpleNamespace(ROLE_PASSWORD_TEXT=1)), patch.object(fill, 'identity', return_value={key:target[key] for key in ('pid','started','path')}), patch.object(fill, 'field_value', return_value=value) as text:
            self.assertTrue(fill.matches(node,target))
            text.return_value='replacement'
            self.assertFalse(fill.matches(node,target))
            text.return_value=''
            self.assertFalse(fill.matches(node,target))
            node.getRole=lambda:1
            self.assertTrue(fill.matches(node,target))
            node.getRole=lambda:0
            text.return_value=value
            self.assertTrue(fill.matches(node,target))
            text.return_value='same-length-edit!'
            self.assertFalse(fill.matches(node,target))

    def test_mp11_masked_insertion_binds_prefix_partial_and_repeated_fill(self):
        for previous, inserted, actual in [('prefix-', 'public-canary', 'public-canary'), ('prefix-public-canary', 'again', 'again'), ('prefix-', 'partial', 'part')]:
            final = previous + actual
            target = {'pid': 200, 'started': 'one', 'path': '/entry', 'registration': 1,
                      'pending': True, 'value_hash': hashlib.sha256(inserted.encode()).hexdigest(), 'length': len(inserted)}
            node = SimpleNamespace(getRole=lambda: 1, queryText=lambda: SimpleNamespace(characterCount=len(final)))
            with patch.dict(sys.modules, pyatspi=SimpleNamespace(ROLE_PASSWORD_TEXT=1)), patch.object(fill, 'identity', return_value={key:target[key] for key in ('pid','started','path')}), patch.object(fill, 'field_value', return_value='•' * len(final)), patch.object(fill, 'update'):
                record = (node, target, (len(previous), len(previous), 0))
                fill.finish(record, inserted)
                self.assertTrue(fill.matches(node, target))
                node.getRole = lambda: 0
                with patch.object(fill, 'field_value', return_value=final):
                    self.assertTrue(fill.matches(node, target), 'MP-11 revealing the same insertion must retain coverage')
                with patch.object(fill, 'field_value', return_value='X' * len(previous) + actual):
                    self.assertFalse(fill.matches(node, target), 'MP-11 same-length replacement retires after reveal')

    def test_mp11_reused_process_object_is_not_the_fill_target(self):
        target={'pid':200,'started':'old','path':'/entry','pending':True}
        with patch.dict(sys.modules, pyatspi=SimpleNamespace(ROLE_PASSWORD_TEXT=1)), patch.object(fill,'identity',return_value={'pid':200,'started':'new','path':'/entry'}):
            self.assertFalse(fill.matches(SimpleNamespace(),target))

    def test_mp11_transactional_store_preserves_newer_fill(self):
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, XDG_RUNTIME_DIR=root, DISPLAY=':test'):
            fill.update(lambda targets: [{'registration':1,'value_hash':'public-fingerprint'}])
            stale=fill.read()
            fill.update(lambda targets: [{'registration':2,'value_hash':'new-fingerprint'}])
            removed={t['registration'] for t in stale}
            fill.update(lambda targets:[t for t in targets if t['registration'] not in removed])
            self.assertEqual(fill.read(),[{'registration':2,'value_hash':'new-fingerprint'}])
            self.assertEqual(fill.store_path().stat().st_mode & 0o777,0o600)

class FillProofTests(unittest.TestCase):
    def test_mp11_unknown_live_field_state_refuses_capture(self):
        target={'pid':200,'started':'one','path':'/entry','value_hash':'public-fingerprint','length':5}
        node=SimpleNamespace(getRole=lambda:0)
        with patch.dict(sys.modules,pyatspi=SimpleNamespace(ROLE_PASSWORD_TEXT=1)),patch.object(fill,'identity',return_value={key:target[key] for key in ('pid','started','path')}),patch.object(fill,'field_value',side_effect=RuntimeError('private diagnostic')):
            with self.assertRaisesRegex(ValueError,'Vault fill field unavailable'):
                fill.matches(node,target)

    def test_mp11_missing_registration_refuses_vault_input(self):
        with patch.dict(sys.modules,pyatspi=None):
            with self.assertRaisesRegex(ValueError,'Vault fill field unavailable'):
                fill.begin(10,'public-disposable-canary')

if __name__=='__main__':unittest.main()
