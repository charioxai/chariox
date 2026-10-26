"""MP-08/MP-11: native field masks and clipboard capture fences."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('native_computer',Path(__file__).with_name('native-computer.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
clipboard=module.load('native-clipboard')
x11=module.load('native-x11')
destination_policy=module.load('native-input-destination')
destination_policy.owner_pid=clipboard.owner_pid
# Real X11 opener (patched Display); every other helper is the given fake.
helpers=lambda accessibility:(lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else clipboard)
class ProtectionTests(unittest.TestCase):
    def test_mp08_mp11_password_dots_leave_desktop_visible_and_only_plaintext_value_is_masked(self):
        # Password roles still protect structured text/input; their dots are safe
        # pixels. Only an explicit recorded plain fill contributes a local mask.
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        for masks in ([],[[2,1,2,2]]):
            with self.subTest(masks=masks):
                tree={'available':True,'complete':True,'protected':True,
                      'nodes':[{'role':'password text','protected':True,'name':'[protected]'}],
                      'masks':masks,'uncovered':[]}
                with patch.object(module,'load',side_effect=helpers(SimpleNamespace(snapshot=lambda *args:tree,capture_snapshot=lambda *args:tree))),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
                    result=module.main({'op':'screenshot','mask':False,'processes':[],'values':['synthetic-only']})
                self.assertFalse(result['protected'])
                with module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64']))) as image:
                    for y in range(4):
                        for x in range(8):
                            covered=bool(masks) and 2<=x<4 and 1<=y<3
                            self.assertEqual(image.getpixel((x,y)),(0,0,0) if covered else (200,200,200))

    def test_mp11_opaque_browser_keeps_unknown_clipboard_contents_withheld(self):
        tree={'available':True,'complete':True,'protected':False,
              'nodes':[{'protected':True,'name':'[protected]'}],'uncovered':[]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree,capture_snapshot=lambda *args:tree)
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else clipboard),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'private-browser-canary')) as read:
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
        read.assert_not_called()

    def test_mp11_r2_unknown_clipboard_source_is_withheld_with_public_coverage(self):
        tree={'available':True,'complete':True,'protected':False,'nodes':[]}
        with patch.object(module,'load',side_effect=lambda name:SimpleNamespace(snapshot=lambda *args:tree,capture_snapshot=lambda *args:tree) if name=='native-accessibility' else x11 if name=='native-x11' else clipboard), patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'unknown-source-canary')):
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})

    def test_mp11_public_coverage_without_a_clipboard_source_remains_withheld(self):
        before={'available':True,'complete':True,'protected':False,'nodes':[]}
        accessibility=SimpleNamespace(snapshot=lambda *args:before,capture_snapshot=lambda *args:before)
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else clipboard),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'public')):
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
    def test_mp08_mp11_only_explicit_fill_masks_affect_pixels(self):
        # Uncovered authority bounds do not contribute visual masks.
        tree={'available':True,'complete':True,'protected':False,'nodes':[],'uncovered':[[2,1,3,2]],'masks':[[2,1,2,2]]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree,capture_snapshot=lambda *args:tree)
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        with patch.object(module,'load',side_effect=helpers(accessibility)),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            self.assertFalse(result['protected'])
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((2,1)),(0,0,0));self.assertEqual(image.getpixel((3,2)),(0,0,0));self.assertEqual(image.getpixel((4,2)),(200,200,200))
            self.assertEqual(image.getpixel((1,1)),(200,200,200));self.assertEqual(image.getpixel((5,1)),(200,200,200));self.assertEqual(image.getpixel((2,3)),(200,200,200))
            # A terminal selection may hold typed secrets: keep the clipboard closed.
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
    def test_mp11_missing_fill_masks_preserves_uncovered_frames(self):
        tree={'available':True,'complete':True,'protected':False,'nodes':[],'uncovered':[[2,1,3,2]]}
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        with patch.object(module,'load',side_effect=helpers(SimpleNamespace(snapshot=lambda *args:tree,capture_snapshot=lambda *args:tree))),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((4,2)),(200,200,200))
    def test_mp08_mp11_browser_protection_fences_both_snapshots_and_reports_withheld_windows(self):
        calls=[]
        def snapshot(*args):
            calls.append(args)
            return {'available':True,'complete':True,'protected':False,'nodes':[],'masks':[[0,0,2,2]],'browser_withheld':1}
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        protection={'pages':[]}
        with patch.object(module,'load',side_effect=helpers(SimpleNamespace(snapshot=snapshot,capture_snapshot=snapshot))),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[],'browser_protection':protection})
        self.assertEqual([call[2] for call in calls],[protection,protection])
        self.assertEqual(result['browser_withheld'],1)

class AgentInputClipboardTests(unittest.TestCase):
    """MP-11: agent input is not gated on who owns the clipboard (reads stay masked)."""
    def run_input(self,action,owner_pid=None,owners=None,tree=None):
        tree=tree or {'available':True,'complete':False,'traversed':True,'protected':False,'uncovered':[[0,0,4,4]],
            'nodes':[{'pid':77,'protected':False},{'pid':90,'protected':True,'name':'[protected]'}]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree,alive=lambda process:True,input_guard=lambda processes:(lambda:None),NativeInputDenied=type('NativeInputDenied',(ValueError,),{}))
        owner=SimpleNamespace(id=99,get_full_property=lambda *args:None)
        destination=SimpleNamespace(id=100,get_full_property=lambda *args:None,query_pointer=lambda:SimpleNamespace(child=0))
        connection=SimpleNamespace(intern_atom=lambda name:name,close=lambda:None,sync=lambda:None,
            get_selection_owner=lambda atom:(owners.pop(0) if owners else (owner if owner_pid else 0)),
            has_extension=lambda name:True,res_query_version=lambda:SimpleNamespace(server_major=1,server_minor=2),
            res_query_client_ids=lambda specs:SimpleNamespace(ids=[SimpleNamespace(spec=SimpleNamespace(mask=2),value=[77 if specs[0]['client']==100 else owner_pid])]),
            get_input_focus=lambda:SimpleNamespace(focus=destination),
            screen=lambda:SimpleNamespace(width_in_pixels=100,height_in_pixels=100,root=SimpleNamespace(query_pointer=lambda:SimpleNamespace(child=destination,root_x=5,root_y=5))))
        events=[]
        def press(kind,value,duration,*args,before_press=None,**kwargs):
            before_press();events.append((kind,value))
        def type_text(text,before_press=None,**kwargs):
            before_press();events.append(('text',text))
        loads={'native-accessibility':accessibility,'native-clipboard':clipboard,'native-x11':x11,'native-input-destination':destination_policy}
        with patch.object(module,'load',side_effect=loads.get),patch.object(module.display,'Display',return_value=connection),patch.object(clipboard,'display_module',return_value=module.display),\
             patch.object(clipboard.subprocess,'run',return_value=SimpleNamespace(stdout=b'public',returncode=0)),\
             patch.object(module.xtest,'fake_input',side_effect=lambda c,kind,*args,**kw:events.append(kind)),\
             patch.object(module.keyboard,'hold_input',side_effect=press),patch.object(module.keyboard,'type_text',side_effect=type_text):
            try:module.input_action(action,[{'pid':77,'started':'1'}])
            except accessibility.NativeInputDenied:return None
        return events

    def test_empty_clipboard_admits_clicks_while_browser_and_popups_are_masked(self):
        self.assertIn(module.X.ButtonPress,self.run_input({'kind':'click','x':5,'y':5}))

    def test_mp11_empty_clipboard_does_not_authorize_popup_free_browser_toolbar_action(self):
        # A popup-free extension runs on ButtonPress without creating a CDP page.
        # The actual destination is Chrome even though another native app has focus.
        node=SimpleNamespace(id=99,get_full_property=lambda *_:None,query_pointer=lambda:SimpleNamespace(child=0))
        root=SimpleNamespace(query_pointer=lambda:SimpleNamespace(child=node,root_x=5,root_y=5))
        connection=SimpleNamespace(intern_atom=lambda name:name,close=lambda:None,sync=lambda:None,get_input_focus=lambda:SimpleNamespace(focus=node),
            get_selection_owner=lambda _:0,screen=lambda:SimpleNamespace(root=root,width_in_pixels=100,height_in_pixels=100),
            has_extension=lambda _:True,res_query_version=lambda:SimpleNamespace(server_major=1,server_minor=2),
            res_query_client_ids=lambda _:SimpleNamespace(ids=[SimpleNamespace(spec=SimpleNamespace(mask=2),value=[77])]))
        accessibility=SimpleNamespace(alive=lambda _:True,input_guard=lambda _:lambda:None,NativeInputDenied=type('NativeInputDenied',(ValueError,),{}))
        def press(*args,before_press=None,**kwargs):before_press()
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else destination_policy if name=='native-input-destination' else clipboard),patch.object(module.display,'Display',return_value=connection),patch.object(clipboard,'display_module',return_value=module.display),patch.object(module.xtest,'fake_input') as event,patch.object(module.keyboard,'hold_input',side_effect=press),patch.object(module.keyboard,'type_text',side_effect=press):
            for action in [{'kind':'click','x':5,'y':5},{'kind':'key','key':'Return'},{'kind':'text','text':' '}]:
                with self.subTest(action=action),self.assertRaisesRegex(ValueError,'Browser'):
                    module.input_action(action,[{'pid':77,'started':'1','browser':True}])
                self.assertFalse(any(call.args[1]==module.X.ButtonPress for call in event.call_args_list))

    def test_agent_written_clipboard_does_not_stop_click_key_text_and_paste(self):
        # The owner is a headless xclip (no accessibility nodes), as after clipboard_write.
        for action in [{'kind':'click','x':5,'y':5},{'kind':'key','key':'ctrl+v'},{'kind':'key','key':'Return'},
                       {'kind':'key','key':'alt+e'},{'kind':'text','text':'p'}]:
            for pid in (90,123):
                self.assertIsNotNone(self.run_input(action,owner_pid=pid),(action,pid))

    def test_clipboard_owner_change_between_presses_does_not_refuse_input(self):
        owner=SimpleNamespace(id=55,get_full_property=lambda *args:None)
        self.assertIsNotNone(self.run_input({'kind':'click','x':5,'y':5},owner_pid=123,owners=[0,owner]))
        self.assertIsNotNone(self.run_input({'kind':'key','key':'p'},owner_pid=123,owners=[0,owner]))

    def test_truncated_owner_walk_does_not_refuse_pointer_or_paste(self):
        tree={'available':True,'complete':False,'traversed':False,'protected':False,'uncovered':[],
            'nodes':[{'pid':77,'protected':False}]}
        for action in [{'kind':'click','x':5,'y':5},{'kind':'key','key':'ctrl+v'}]:
            self.assertIsNotNone(self.run_input(action,owner_pid=77,tree=tree),action)

class AgentHoldTests(unittest.TestCase):
    def test_agent_pointer_and_key_holds_reach_the_hold_helper(self):
        accessibility=SimpleNamespace(input_guard=lambda _:lambda:None)
        clipboard_guard=SimpleNamespace(key_destinations=lambda *_:None)
        with patch.object(module.keyboard,'hold_input') as hold,patch.object(module,'load',side_effect=lambda name:accessibility if name=='native-accessibility' else clipboard_guard):
            module.main({'op':'input','agent_input':True,'processes':[],'input':{'kind':'pointer_hold','button':2,'x':5,'y':6,'duration_ms':100}})
            module.main({'op':'input','agent_input':True,'processes':[],'input':{'kind':'hold','key':'a','duration_ms':100}})
        self.assertEqual([call.args[:2] for call in hold.call_args_list],[('button','middle'),('key','a')])

class WarmChannelTests(unittest.TestCase):
    def test_mp10_mp11_human_committed_text_is_unpaced_but_agent_text_stays_fenced(self):
        connection=object()
        with patch.object(module.keyboard,'type_text') as text:
            module.channel_request({'op':'input','input':{'kind':'text','text':'m'*128}},set(),connection)
            self.assertEqual(text.call_args.kwargs.get('pace_seconds',0.04),0)
            self.assertIs(text.call_args.kwargs.get('connection'),connection)
            accessibility=SimpleNamespace(input_guard=lambda _:lambda:None)
            clipboard_guard=SimpleNamespace(key_destinations=lambda *_:None)
            with patch.object(module,'load',side_effect=lambda name:accessibility if name=='native-accessibility' else clipboard_guard):
                module.main({'op':'input','agent_input':True,'processes':[],
                             'input':{'kind':'text','text':'public'}},connection)
            self.assertIsNotNone(text.call_args.kwargs['before_press'])
            self.assertIs(text.call_args.kwargs['key_guard'],clipboard_guard.key_destinations)
            self.assertEqual(text.call_args.kwargs.get('pace_seconds',0.04),0.04)
            self.assertNotIn('connection',text.call_args.kwargs)

    def test_mp10_warm_physical_events_reuse_the_owned_x11_connection(self):
        connection=SimpleNamespace(screen=lambda:SimpleNamespace(width_in_pixels=100,height_in_pixels=100),sync=lambda:None)
        with patch.object(module,'load') as load,patch.object(module.xtest,'fake_input') as event:
            held=set()
            for action in ({'kind':'scroll','x':5,'y':5,'steps':1},
                           {'kind':'keycode','keycode':38,'state':'down'},
                           {'kind':'keycode','keycode':38,'state':'up'}):
                module.channel_request({'op':'input','input':action},held,connection)
            load.assert_not_called()
            self.assertTrue(event.called)
            self.assertEqual(held,set())

    def test_mp11_warm_chord_and_text_restore_termination_between_events(self):
        # The shared chord helper shields key-up restoration. That shield must
        # not survive a request in the long-lived human input process.
        original = {number: module.signal.getsignal(number) for number in
                    (module.signal.SIGTERM, module.signal.SIGINT)}
        def terminate(number, frame):
            raise SystemExit(128 + number)
        try:
            for kind, helper, action in (
                    ('key', 'hold_input', {'kind': 'key', 'key': 'Home'}),
                    ('text', 'type_text', {'kind': 'text', 'text': 'Grüße 世界😀'})):
                for failed in (False, True):
                    with self.subTest(kind=kind, failed=failed):
                        for number in original:
                            module.signal.signal(number, terminate)
                        def dispatch(*args, **kwargs):
                            for number in original:
                                module.signal.signal(number, module.signal.SIG_IGN)
                            if failed:
                                raise ValueError('input failed after key-up shield')
                        with patch.object(module.keyboard, helper, side_effect=dispatch):
                            request = {'op': 'input', 'input': action}
                            if failed:
                                with self.assertRaises(ValueError):
                                    module.channel_request(request, set())
                            else:
                                module.channel_request(request, set())
                        for number in original:
                            self.assertIs(module.signal.getsignal(number), terminate)
        finally:
            for number, handler in original.items():
                module.signal.signal(number, handler)

    def test_mp11_warm_channel_carries_only_human_input(self):
        events=[]
        with patch.object(module,'input_action',side_effect=lambda action,processes,connection=None,browser_page=None:events.append((action['kind'],processes))):
            held=set()
            for request in ({'op':'input','agent_input':True,'processes':[],'input':{'kind':'click','x':1,'y':1}},
                            {'op':'input','processes':[],'input':{'kind':'key','key':'ctrl+v'}},
                            {'op':'input','agent_input':True,'processes':[],'input':{'kind':'text','text':'x'}},
                            {'op':'input','input':{'kind':'text','text':'x'*129}},
                            {'op':'input','input':{'kind':'clipboard_write','text':'x'}},
                            {'op':'accessibility','processes':[]}):
                with self.assertRaises(ValueError):module.channel_request(request,held)
            self.assertEqual(events,[])
            for kind,extra in (('click',{'x':1,'y':1}),('scroll',{'x':1,'y':1,'steps':1}),('key',{'key':'Next'}),('keycode',{'keycode':38,'state':'down'}),('text',{'text':'Grüße 世界😀'})):
                module.channel_request({'op':'input','input':{'kind':kind,**extra}},held)
            self.assertEqual(events,[('click',None),('scroll',None),('key',None),('keycode',None),('text',None)])
            self.assertEqual(held,{38})
class CaptureEvidenceTests(unittest.TestCase):
    def test_mp08_mp11_capture_uses_fill_evidence_and_keeps_input_authority_separate(self):
        from unittest.mock import Mock
        strict = Mock(side_effect=AssertionError('input authority must not define capture coverage'))
        fields = Mock(return_value={'available':True,'complete':True,'masks':[],'browser_withheld':0})
        accessibility = SimpleNamespace(snapshot=strict,capture_snapshot=fields)
        with patch.object(module,'load',return_value=accessibility),patch.object(module,'capture',return_value=module.Image.new('RGB',(8,4),'white')):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
        strict.assert_not_called()
        self.assertEqual(fields.call_count,2)
        self.assertFalse(result['protected'])


if __name__=='__main__':unittest.main()
