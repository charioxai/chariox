"""MP-11: clipboard reads must fence native protection changes during the read."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('native_computer',Path(__file__).with_name('native-computer.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
clipboard=module.load('native-clipboard')
x11=module.load('native-x11')
class ProtectionTests(unittest.TestCase):
    def test_mp11_opaque_browser_keeps_unknown_clipboard_contents_withheld(self):
        tree={'available':True,'complete':True,'protected':False,
              'nodes':[{'protected':True,'name':'[protected]'}],'uncovered':[]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree)
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else clipboard),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'private-browser-canary')) as read:
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
        read.assert_not_called()

    def test_mp11_r2_unknown_clipboard_source_is_withheld_with_public_coverage(self):
        tree={'available':True,'complete':True,'protected':False,'nodes':[]}
        with patch.object(module,'load',side_effect=lambda name:SimpleNamespace(snapshot=lambda *args:tree) if name=='native-accessibility' else x11 if name=='native-x11' else clipboard), patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'unknown-source-canary')):
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})

    def test_mp11_public_coverage_without_a_clipboard_source_remains_withheld(self):
        before={'available':True,'complete':True,'protected':False,'nodes':[]}
        accessibility=SimpleNamespace(snapshot=lambda *args:before)
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else clipboard),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'public')):
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
    def test_mp08_uncovered_owned_window_is_blacked_out_not_the_desktop(self):
        # Owned app stacked above covers x>=4 of the terminal frame: only the exposed part is blacked out.
        tree={'available':True,'complete':True,'protected':False,'nodes':[],'uncovered':[[2,1,3,2]],'masks':[[2,1,2,2]]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree)
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else clipboard),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            self.assertFalse(result['protected'])
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((2,1)),(0,0,0));self.assertEqual(image.getpixel((3,2)),(0,0,0));self.assertEqual(image.getpixel((4,2)),(200,200,200))
            self.assertEqual(image.getpixel((1,1)),(200,200,200));self.assertEqual(image.getpixel((5,1)),(200,200,200));self.assertEqual(image.getpixel((2,3)),(200,200,200))
            # A terminal selection may hold typed secrets: keep the clipboard closed.
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
    def test_mp11_missing_masks_falls_back_to_whole_uncovered_frames(self):
        tree={'available':True,'complete':True,'protected':False,'nodes':[],'uncovered':[[2,1,3,2]]}
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        with patch.object(module,'load',side_effect=lambda name:SimpleNamespace(snapshot=lambda *args:tree) if name=='native-accessibility' else x11 if name=='native-x11' else clipboard),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((4,2)),(0,0,0))

class AgentInputClipboardTests(unittest.TestCase):
    """MP-11 #904 review 1/3: every agent mutation is gated on CLIPBOARD owner provenance only."""
    def run_input(self,action,owner_pid=None,owners=None,tree=None):
        tree=tree or {'available':True,'complete':False,'traversed':True,'protected':False,'uncovered':[[0,0,4,4]],
            'nodes':[{'pid':77,'protected':False},{'pid':90,'protected':True,'name':'[protected]'}]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree,alive=lambda process:True,input_guard=lambda processes:(lambda:None),NativeInputDenied=type('NativeInputDenied',(ValueError,),{}))
        owner=SimpleNamespace(id=99,get_full_property=lambda *args:None)
        connection=SimpleNamespace(intern_atom=lambda name:name,close=lambda:None,sync=lambda:None,
            get_selection_owner=lambda atom:(owners.pop(0) if owners else (owner if owner_pid else 0)),
            has_extension=lambda name:True,res_query_version=lambda:SimpleNamespace(server_major=1,server_minor=2),
            res_query_client_ids=lambda specs:SimpleNamespace(ids=[SimpleNamespace(spec=SimpleNamespace(mask=2),value=[owner_pid])]),
            screen=lambda:SimpleNamespace(width_in_pixels=100,height_in_pixels=100))
        events=[]
        def press(kind,value,duration,*args,before_press=None):
            before_press();events.append((kind,value))
        def type_text(text,before_press=None):
            before_press();events.append(('text',text))
        loads={'native-accessibility':accessibility,'native-clipboard':clipboard,'native-x11':x11}
        with patch.object(module,'load',side_effect=loads.get),patch.object(module.display,'Display',return_value=connection),patch.object(clipboard,'display_module',return_value=module.display),\
             patch.object(clipboard.subprocess,'run',return_value=SimpleNamespace(stdout=b'public',returncode=0)),\
             patch.object(module.xtest,'fake_input',side_effect=lambda c,kind,*args,**kw:events.append(kind)),\
             patch.object(module.keyboard,'hold_input',side_effect=press),patch.object(module.keyboard,'type_text',side_effect=type_text):
            try:module.input_action(action,[{'pid':77,'started':'1'}])
            except accessibility.NativeInputDenied:return None
        return events

    def test_empty_clipboard_admits_clicks_while_browser_and_popups_are_masked(self):
        self.assertIn(module.X.ButtonPress,self.run_input({'kind':'click','x':5,'y':5}))

    def test_proved_public_owner_admits_clicks_while_other_windows_are_masked(self):
        self.assertIn(module.X.ButtonPress,self.run_input({'kind':'click','x':5,'y':5},owner_pid=77))

    def test_unproved_or_protected_owner_refuses_every_mutating_input_before_events(self):
        for action in [{'kind':'click','x':5,'y':5},{'kind':'key','key':'p'},{'kind':'key','key':'Return'},
                       {'kind':'key','key':'alt+e'},{'kind':'key','key':'shift+F10'},{'kind':'text','text':'p'}]:
            for pid in (90,123):
                self.assertIsNone(self.run_input(action,owner_pid=pid),(action,pid))

    def test_mp11_review1_truncated_owner_walk_refuses_pointer_paste_before_events(self):
        tree={'available':True,'complete':False,'traversed':False,'protected':False,'uncovered':[],
            'nodes':[{'pid':77,'protected':False}]}
        for action in [{'kind':'click','x':5,'y':5},{'kind':'key','key':'ctrl+v'}]:
            self.assertIsNone(self.run_input(action,owner_pid=77,tree=tree),action)

    def test_owner_taken_after_admission_refuses_the_press(self):
        owner=SimpleNamespace(id=55,get_full_property=lambda *args:None)
        self.assertIsNone(self.run_input({'kind':'click','x':5,'y':5},owner_pid=123,owners=[0,owner]))
        self.assertIsNone(self.run_input({'kind':'key','key':'p'},owner_pid=123,owners=[0,owner]))
if __name__=='__main__':unittest.main()
