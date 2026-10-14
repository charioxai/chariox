"""MP-11 R2: source provenance, coverage and change fences for reads/paste."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
spec=importlib.util.spec_from_file_location('native_clipboard',Path(__file__).with_name('native-clipboard.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class ClipboardTests(unittest.TestCase):
    def setup(self):
        process={'pid':77,'started':'1'}
        tree={'available':True,'complete':True,'traversed':True,'protected':False,'nodes':[{'pid':77,'protected':False}]}
        access=SimpleNamespace(snapshot=Mock(return_value=tree),alive=Mock(return_value=True),NativeInputDenied=ValueError)
        owner=SimpleNamespace(id=99,get_full_property=Mock(return_value=SimpleNamespace(format=32,value=[77])))
        connection=SimpleNamespace(get_selection_owner=Mock(return_value=owner),intern_atom=lambda name:name,close=Mock(),
            has_extension=Mock(return_value=True),res_query_version=Mock(return_value=SimpleNamespace(server_major=1,server_minor=2)),
            res_query_client_ids=Mock(return_value=SimpleNamespace(ids=[SimpleNamespace(spec=SimpleNamespace(mask=2),value=[77])])))
        return process,tree,access,owner,connection

    def test_unknown_selection_owner_never_reads_clipboard_contents(self):
        p,t,a,o,c=self.setup();o.get_full_property.return_value=None;c.res_query_client_ids.return_value=SimpleNamespace(ids=[])
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run') as read:
            self.assertIsNone(m.public_clipboard([p],a));read.assert_not_called()
        c.close.assert_called_once()

    def test_gtk_selection_window_can_use_proved_server_pid_without_a_client_claim(self):
        p,t,a,o,c=self.setup();o.get_full_property.return_value=None
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=b'public')):
            self.assertEqual(m.public_clipboard([p],a),('public',(99,77,'1')))

    def test_foreign_or_retired_clipboard_source_is_protected(self):
        p,t,a,o,c=self.setup()
        for processes,alive in [([],True),([p],False),([{'pid':78,'started':'1'}],True)]:
            a.alive.return_value=alive
            with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run') as read:
                self.assertIsNone(m.public_clipboard(processes,a));read.assert_not_called()

    def test_forged_public_pid_is_not_clipboard_provenance(self):
        p,t,a,o,c=self.setup()
        c.has_extension=Mock(return_value=True)
        c.res_query_version=Mock(return_value=SimpleNamespace(server_major=1,server_minor=2))
        c.res_query_client_ids=Mock(return_value=SimpleNamespace(ids=[SimpleNamespace(spec=SimpleNamespace(mask=2),value=[78])]))
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run') as read:
            self.assertIsNone(m.public_clipboard([p],a));read.assert_not_called()

    def test_missing_or_unsupported_server_provenance_never_reads_bytes(self):
        p,t,a,o,c=self.setup()
        for version in [(1,1),(1,2)]:
            c.res_query_version.return_value=SimpleNamespace(server_major=version[0],server_minor=version[1])
            c.res_query_client_ids.return_value=SimpleNamespace(ids=[])
            with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run') as read:
                self.assertIsNone(m.public_clipboard([p],a));read.assert_not_called()
        c.has_extension.return_value=False
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run') as read:
            self.assertIsNone(m.public_clipboard([p],a));read.assert_not_called()

    def test_complete_public_owned_source_allows_the_same_read_and_paste_policy(self):
        p,t,a,o,c=self.setup()
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=b'public',returncode=0)):
            self.assertEqual(m.public_clipboard([p],a),('public',(99,77,'1')))
            m.input_admission([p],a)()

    def test_mp11_review1_truncated_owner_traversal_is_not_a_public_source(self):
        # A public owner node is retained, its password subtree was omitted by the walk budget.
        p,t,a,o,c=self.setup();a.snapshot.return_value={**t,'complete':False,'traversed':False}
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run') as read:
            with self.assertRaises(ValueError):m.input_admission([p],a)
            self.assertIsNone(m.public_clipboard([p],a))
        read.assert_not_called()

    def test_mp11_review2_same_owner_becoming_protected_or_replacing_contents_refuses_next_press(self):
        p,t,a,o,c=self.setup()
        protected={**t,'nodes':[{'pid':77,'protected':False},{'pid':77,'protected':True}]}
        # (snapshots, selection (TIMESTAMP, text) at admission then at the press); owner id/PID never change.
        for snapshots,selections in [([t,protected],[(b'1',b'a'),(b'1',b'a')]),
                                     ([t,t],[(b'1',b'a'),(b'2',b'a')]),
                                     ([t,t],[(b'1',b'a'),(b'1',b'private')])]:
            a.snapshot.side_effect=snapshots;current={}
            run=lambda args,**kw:SimpleNamespace(stdout=current['value'][0] if 'TIMESTAMP' in args else current['value'][1],returncode=0)
            with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run',side_effect=run):
                current['value']=selections[0];check=m.input_admission([p],a)
                current['value']=selections[1]
                with self.assertRaises(ValueError):check()
        # Unchanged owner, protection and selection stay admitted press after press.
        a.snapshot.side_effect=None;a.snapshot.return_value=t
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=b'same',returncode=0)):
            check=m.input_admission([p],a);check();check()

    def test_mp11_empty_clipboard_admits_input_without_reading_or_traversing(self):
        p,t,a,o,c=self.setup();c.get_selection_owner.return_value=0
        a.snapshot.return_value={**t,'complete':False,'uncovered':[[0,0,9,9]]}
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run') as read:
            m.input_admission([p],a)()
        read.assert_not_called();a.snapshot.assert_not_called()

    def test_mp11_public_owner_admits_input_while_other_windows_are_masked(self):
        p,t,a,o,c=self.setup()
        a.snapshot.return_value={**t,'complete':False,'protected':True,'uncovered':[[0,0,9,9]],'nodes':[{'pid':77,'protected':False},{'pid':90,'protected':True}]}
        with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=b'public',returncode=0)):
            m.input_admission([p],a)()

    def test_mp11_unproved_or_protected_owner_refuses_input(self):
        p,t,a,o,c=self.setup()
        for tree,pid in [(t,78),({**t,'nodes':[{'pid':77,'protected':False},{'pid':77,'protected':True}]},77),({**t,'nodes':[]},77)]:
            a.snapshot.return_value=tree
            c.res_query_client_ids.return_value=SimpleNamespace(ids=[SimpleNamespace(spec=SimpleNamespace(mask=2),value=[pid])])
            with patch('Xlib.display.Display',return_value=c):
                with self.assertRaises(ValueError):m.input_admission([p],a)

    def test_mp11_owner_change_is_refused_before_the_next_press(self):
        p,t,a,o,c=self.setup()
        other=SimpleNamespace(id=55,get_full_property=Mock(return_value=None))
        for owners in [[0,o],[o,o,0],[o,o,other]]:
            c.get_selection_owner.side_effect=owners
            with patch('Xlib.display.Display',return_value=c),patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=b'public',returncode=0)):
                check=m.input_admission([p],a)
                with self.assertRaises(ValueError):check()

    def test_registry_or_native_protection_never_reads_bytes(self):
        p,t,a,o,c=self.setup()
        for tree,mask in [(t,True),({**t,'protected':True},False),({**t,'complete':False},False),({**t,'nodes':[{'pid':77,'protected':True}]},False)]:
            a.snapshot.return_value=tree
            with patch('Xlib.display.Display') as connect,patch.object(m.subprocess,'run') as read:
                self.assertIsNone(m.public_clipboard([p],a,mask));connect.assert_not_called();read.assert_not_called()

if __name__=='__main__':unittest.main()
