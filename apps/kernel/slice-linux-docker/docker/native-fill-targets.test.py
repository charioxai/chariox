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
            with patch.dict(sys.modules, pyatspi=SimpleNamespace(ROLE_PASSWORD_TEXT=1)), patch.object(fill, 'identity', return_value={key:target[key] for key in ('pid','started','path')}), patch.object(fill, 'field_value', return_value='•' * len(final)), patch.object(fill, 'update'), patch.object(fill.time, 'sleep'):
                record = (node, target, (len(previous), len(previous), 0))
                fill.finish(record, inserted)
                self.assertTrue(fill.matches(node, target))
                node.getRole = lambda: 0
                with patch.object(fill, 'field_value', return_value=final):
                    self.assertTrue(fill.matches(node, target), 'MP-11 revealing the same insertion must retain coverage')
                if actual != inserted:
                    self.assertTrue(target['pending'], 'MP-11 truncation cannot be distinguished from queued input')
                    continue
                with patch.object(fill, 'field_value', return_value='X' * len(previous) + actual):
                    self.assertFalse(fill.matches(node, target), 'MP-11 same-length replacement retires after reveal')

    def test_mp11_delayed_delivery_stays_pending_and_masks_plain_field(self):
        for password in [False, True]:
            value = 'prefix-'
            inserted = 'public-delayed-canary'
            target = {'pid': 200, 'started': 'one', 'path': '/entry', 'window': 10,
                      'registration': 1, 'pending': True}
            node = SimpleNamespace(path='/entry', childCount=0, parent=None,
                get_process_id=lambda: 200, getRole=lambda: int(password),
                queryText=lambda: SimpleNamespace(characterCount=len(value), getText=lambda a,b: value),
                getState=lambda: SimpleNamespace(contains=lambda state: True),
                queryComponent=lambda: SimpleNamespace(getExtents=lambda mode: SimpleNamespace(x=20,y=30,width=100,height=20)))
            node.getRoleName = lambda: 'entry'
            desktop = SimpleNamespace(childCount=1, getChildAtIndex=lambda i: node)
            window = SimpleNamespace(get_full_property=lambda *args: SimpleNamespace(value=[200]),
                get_attributes=lambda: SimpleNamespace(map_state=2))
            connection = SimpleNamespace(create_resource_object=lambda *args: window,
                intern_atom=lambda *args: 1, close=lambda: None,
                screen=lambda: SimpleNamespace(width_in_pixels=800,height_in_pixels=600))
            xlib=SimpleNamespace(X=SimpleNamespace(AnyPropertyType=0,IsViewable=2),display=SimpleNamespace(),error=SimpleNamespace(BadWindow=type('BadWindow',(Exception,),{})))
            with self.subTest(password=password), patch.dict(sys.modules, pyatspi=SimpleNamespace(ROLE_PASSWORD_TEXT=1,STATE_SHOWING=2,DESKTOP_COORDS=0,Registry=SimpleNamespace(getDesktop=lambda i:desktop)),Xlib=xlib), patch.object(fill,'identity',return_value={key:target[key] for key in ('pid','started','path')}), patch.object(fill,'update'), patch.object(fill.time,'sleep'), patch.object(fill,'read',return_value=[target]), patch.object(fill,'open_display',return_value=connection):
                fill.finish((node,target,(len(value),len(value),0)),inserted)
                self.assertTrue(target['pending'], 'MP-11 deadline is not application acknowledgement')
                value += inserted
                self.assertEqual(fill.regions(), [] if password else [[18,28,104,24]])
                node.getRole=lambda: 0
                self.assertEqual(fill.regions(), [[18,28,104,24]], 'MP-11 delayed fill survives reveal and repeated capture')
                value='user replacement'
                self.assertEqual(fill.regions(), [], 'MP-11 confirmed delivery still retires user replacement')

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
    def test_mp11_incomplete_rediscovery_keeps_the_registration(self):
        target={'pid':200,'started':'one','path':'/entry','window':10,'registration':1}
        missing=SimpleNamespace(childCount=0)
        leaf=SimpleNamespace(path='/other',childCount=0)
        app=SimpleNamespace(path='/app',childCount=8193,get_process_id=lambda:200,getChildAtIndex=lambda i:leaf)
        large=SimpleNamespace(childCount=1,getChildAtIndex=lambda i:app)
        unknown_app=SimpleNamespace(path='/app',childCount=1,get_process_id=lambda:200,getChildAtIndex=lambda i:None)
        unavailable_child=SimpleNamespace(childCount=1,getChildAtIndex=lambda i:unknown_app)
        window=SimpleNamespace(get_full_property=lambda *args:SimpleNamespace(value=[200]),get_attributes=lambda:SimpleNamespace(map_state=2))
        connection=SimpleNamespace(create_resource_object=lambda *args:window,intern_atom=lambda *args:1,close=lambda:None)
        xlib=SimpleNamespace(X=SimpleNamespace(AnyPropertyType=0,IsViewable=2),display=SimpleNamespace(),error=SimpleNamespace(BadWindow=type('BadWindow',(Exception,),{})))
        for desktop in [missing,large,unavailable_child]:
            with self.subTest(app_missing=desktop is missing),patch.dict(sys.modules,pyatspi=SimpleNamespace(Registry=SimpleNamespace(getDesktop=lambda i:desktop)),Xlib=xlib),patch.object(fill,'open_display',return_value=connection),patch.object(fill,'read',return_value=[target]),patch.object(fill,'update') as update:
                with self.assertRaisesRegex(ValueError,'Vault fill field unavailable'):fill.regions()
                update.assert_not_called()
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
