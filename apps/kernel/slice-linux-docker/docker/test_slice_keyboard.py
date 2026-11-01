"""Unicode text must not become Chromium's physical browser accelerators."""
import io
import importlib.util
from pathlib import Path
import sys
from types import SimpleNamespace, ModuleType
import unittest
from unittest.mock import patch, MagicMock


class Display:
    def __init__(self):
        self.text = ""
        self.reloads = 0
        self.mapping = {}
        self.delivered = []
    def query_keymap(self): return bytes(32)
    def get_modifier_mapping(self): return [[]]
    def sync(self): pass
    def close(self): pass


class UpstreamKeyboard:
    """Model the upstream allocation contract and Chromium's native refresh."""
    def __init__(self, connection):
        self.connection = connection
        self.overlay = {}
        self._spare_set = frozenset()
    def _find_spare_keycodes(self):
        # The unassigned contiguous browser-key run wins before isolated slots.
        slots = [181, 182, 183, 8, 93, 97, 103]
        self._spare_set = frozenset(slots)
        return slots
    def layout_carries(self, keysym): return keysym in self.connection.mapping
    def prebind(self, keysyms):
        slots = self._find_spare_keycodes()
        if len(set(keysyms)) > len(slots): return False
        for keysym in dict.fromkeys(keysyms): self._overlay_keycode(keysym)
        return True
    def _overlay_keycode(self, keysym):
        if keysym not in self.overlay:
            slots = self._find_spare_keycodes()
            if not slots: raise ValueError("no slots")
            self.overlay[keysym] = slots[len(self.overlay) % len(slots)]
        return self.overlay[keysym]
    def _resolve(self, keysym):
        return (self.connection.mapping.get(keysym) or self._overlay_keycode(keysym), (), None)
    def press(self, keysym):
        code, _, _ = self._resolve(keysym)
        self.connection.delivered.append(code)
        if code == 181:
            self.connection.reloads += 1
            self.connection.text = ""
        else: self.connection.text += chr(keysym & 0xffffff)
    def release(self, keysym): pass
    def release_group_lock(self): pass


def helper(connection):
    selkies = ModuleType("selkies")
    xlib = ModuleType("selkies.Xlib")
    xlib.X = SimpleNamespace(KeyRelease=3, KeyPress=2, ButtonPress=4, ButtonRelease=5, MotionNotify=6)
    xlib.XK = SimpleNamespace(string_to_keysym=lambda _: 0)
    xdisplay = ModuleType("selkies.Xlib.display")
    xdisplay.Display = lambda: connection
    ext = ModuleType("selkies.Xlib.ext")
    ext.xtest = SimpleNamespace(fake_input=lambda *args: None)
    inputs = ModuleType("selkies.input_handler")
    inputs._XTestKeyboard = UpstreamKeyboard
    inputs.character_to_layout_keysym = lambda character: 0x01000000 | ord(character)
    inputs.universal_text_keysym = inputs.character_to_layout_keysym
    selkies.Xlib = xlib
    modules = {"selkies": selkies, "selkies.Xlib": xlib, "selkies.Xlib.display": xdisplay,
               "selkies.Xlib.ext": ext, "selkies.input_handler": inputs}
    with patch.dict(sys.modules, modules), patch("logging.disable"):
        spec = importlib.util.spec_from_file_location("keyboard_under_test", Path(__file__).with_name("slice-keyboard.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    return module


class KeyboardTextTests(unittest.TestCase):
    def test_delivery_acknowledgement_runs_after_the_protected_server_grab(self):
        connection=Display();connection.grabbed=False;module=helper(connection)
        connection.grab_server=lambda:setattr(connection,'grabbed',True)
        connection.ungrab_server=lambda:setattr(connection,'grabbed',False)
        target={'window':'owned'};acknowledged=[]
        def settle(keyboard):
            self.assertFalse(connection.grabbed,'the client cannot acknowledge under the server grab')
            acknowledged.append(connection.text)
        with patch.object(module,'focused_target',return_value=target),patch.object(module,'assert_secret_target'),patch.object(module.ComputerTextKeyboard,'settle_text',settle,create=True),patch.object(module.signal,'signal'):
            module.type_text('漢字🧪',before_press=lambda:None,pace_seconds=0,connection=connection)
        self.assertEqual(acknowledged,['漢','漢字','漢字🧪'])

    def test_mp08_warm_human_text_borrows_without_reopening_or_closing_x11(self):
        connection=Display()
        module=helper(connection)
        with patch.object(module._x11_module,'open_display',side_effect=AssertionError('warm text reopened X11')),patch.object(connection,'close') as close,patch.object(module.time,'sleep'),patch.object(module.signal,'signal'):
            module.type_text('ab',pace_seconds=0,connection=connection)
        self.assertEqual(connection.text,'ab')
        close.assert_not_called()

    def test_mp11_failed_borrowed_text_leaves_the_channel_connection_owned(self):
        connection=Display()
        module=helper(connection)
        with patch.object(module._x11_module,'open_display',side_effect=AssertionError('warm text reopened X11')),patch.object(connection,'close') as close,patch.object(module,'universal_text_keysym',return_value=None),patch.object(module.signal,'signal'):
            with self.assertRaisesRegex(ValueError,'unsupported text character'):
                module.type_text('a',pace_seconds=0,connection=connection)
        close.assert_not_called()
        self.assertEqual(connection.text,'')

    def test_mp11_one_shot_text_still_closes_its_owned_x11_connection(self):
        connection=Display()
        module=helper(connection)
        with patch.object(module._x11_module,'open_display',return_value=connection) as opened,patch.object(connection,'close') as close,patch.object(module.time,'sleep'),patch.object(module.signal,'signal'):
            module.type_text('ab')
        opened.assert_called_once()
        close.assert_called_once()
        self.assertEqual(connection.text,'ab')

    # Review #1016 @52a7bb1ee P3: text with a key guard probes the root grabs; probe and press share
    # one server grab so no client can grab the key between them, also without a focus target (the
    # native-computer agent path passes before_press, whose focus target already takes the grab).
    def test_mp11_text_key_guard_and_press_share_the_server_grab(self):
        connection=Display();connection.grabbed=False;module=helper(connection)
        connection.grab_server=lambda:setattr(connection,'grabbed',True)
        connection.ungrab_server=lambda:setattr(connection,'grabbed',False)
        probes=[]
        def guard(conn,codes):probes.append(connection.grabbed)
        with patch.object(module.time,'sleep'),patch.object(module.signal,'signal'):
            module.type_text('ab',pace_seconds=0,connection=connection,key_guard=guard)
        self.assertEqual(probes,[True,True])
        self.assertEqual(connection.text,'ab')
        self.assertFalse(connection.grabbed)

    def test_keymap_client_can_reply_before_the_protected_server_grab(self):
        connection=Display();connection.grabbed=False
        connection.grab_server=lambda:setattr(connection,'grabbed',True)
        connection.ungrab_server=lambda:setattr(connection,'grabbed',False)
        module=helper(connection)
        original=UpstreamKeyboard._overlay_keycode
        def bind(keyboard, keysym):
            if keysym not in keyboard.overlay:
                self.assertFalse(connection.grabbed, 'client keymap request blocked by the X server grab')
            return original(keyboard, keysym)
        with patch.object(UpstreamKeyboard,'_overlay_keycode',bind),patch.object(module,'focused_target',return_value={'focus_window':10}),patch.object(module,'assert_secret_target'),patch.object(module.time,'sleep'),patch.object(module.signal,'signal'):
            module.type_text('漢字🧪',before_press=lambda:None)
        self.assertEqual(connection.text,'漢字🧪')
        self.assertFalse(connection.grabbed)

    def test_protection_change_during_mapping_wait_refuses_before_any_press(self):
        connection=Display();connection.protected=False
        connection.grab_server=lambda:None;connection.ungrab_server=lambda:None
        module=helper(connection);original=UpstreamKeyboard._overlay_keycode
        def bind(keyboard, keysym):
            if keysym not in keyboard.overlay:connection.protected=True
            return original(keyboard, keysym)
        def admit():
            if connection.protected:raise ValueError('protected focus changed during keymap read')
        with patch.object(UpstreamKeyboard,'_overlay_keycode',bind),patch.object(module,'focused_target',return_value={'focus_window':10}),patch.object(module,'assert_secret_target'),patch.object(module.time,'sleep'),patch.object(module.signal,'signal'):
            with self.assertRaisesRegex(ValueError,'protected focus changed'):
                module.type_text('漢字🧪',before_press=admit)
        self.assertEqual(connection.text,'')

    def test_mp11_finding1_focus_change_stops_a_multi_character_native_event(self):
        connection=Display()
        connection.grab_server=lambda:None
        connection.ungrab_server=lambda:None
        module=helper(connection)
        checks=[]
        def admit():
            checks.append(True)
            if len(checks)==3:raise ValueError('protected focus changed')
        with patch.object(module,'focused_target',return_value={'focus_window':10}),patch.object(module,'assert_secret_target'),patch.object(module.time,'sleep'),patch.object(module.signal,'signal'):
            with self.assertRaises(ValueError):module.type_text('ab',before_press=admit)
        self.assertEqual(connection.text,'')

    def test_mp11_room_text_cli_refuses_before_a_protected_field_receives_input(self):
        import ast
        connection = Display()
        module = helper(connection)
        source = Path(__file__).with_name('slice-keyboard.py').read_text()
        tree = ast.parse(source)
        entry = next(node for node in tree.body if isinstance(node, ast.If) and '__name__' in ast.unparse(node.test))
        code = compile(ast.Module(body=[entry], type_ignores=[]), str(Path(__file__).with_name('slice-keyboard.py')), 'exec')
        module.__dict__['__name__'] = '__main__'
        with patch.dict(module.os.environ, {'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), \
             patch.object(module.sys, 'argv', ['slice-keyboard.py']), \
             patch.object(module.sys, 'stdin', SimpleNamespace(buffer=io.BytesIO(b'private-canary'))), \
             patch.object(module, 'room_input_guard', create=True, side_effect=ValueError('protected native focus')), \
             patch.object(module.time, 'sleep'), patch.object(module.signal, 'signal'), \
             patch.object(module.sys, 'stderr', io.StringIO()):
            with self.assertRaises(SystemExit):exec(code, module.__dict__)
        self.assertEqual(connection.text, '')

    def type(self, text, inherited=False):
        connection = Display()
        if inherited: connection.mapping[0x01000000 | ord(text[0])] = 181
        module = helper(connection)
        with patch.object(module.time, "sleep"), patch.object(module.signal, "signal"):
            module.type_text(text)
        self.assertEqual(connection.reloads, 0, "Unicode activated Chromium BrowserRefresh")
        self.assertEqual(connection.text, text)
        return connection

    def test_first_unicode_batch_cannot_reload_the_app(self):
        self.type("日本語")

    def test_previous_unsafe_overlay_is_not_reused_as_a_layout_binding(self):
        self.type("日", inherited=True)

    def test_no_safe_slot_fails_without_pressing_an_accelerator(self):
        connection = Display()
        module = helper(connection)
        with patch.object(UpstreamKeyboard, "_find_spare_keycodes", return_value=[181, 182]), \
             patch.object(module.time, "sleep"), patch.object(module.signal, "signal"):
            with self.assertRaises(ValueError): module.type_text("日")
        self.assertEqual(connection.delivered, [])
        self.assertEqual(connection.reloads, 0)

    def test_more_unicode_symbols_than_the_safe_pool_recycle_without_navigation(self):
        connection = self.type("日本語甲乙丙丁戊己庚辛壬")
        self.assertLess(len(set(connection.delivered)), len(connection.delivered))


class OwnedKeymapTests(unittest.TestCase):
    def connection(self):
        c=MagicMock();c.query_keymap.return_value=bytes(32)
        modifiers=[[50],[8,92],[],[],[],[],[],[]]
        c.get_modifier_mapping.side_effect=lambda:modifiers
        def set_modifiers(rows):
            modifiers[:]=rows
            return 0
        c.set_modifier_mapping.side_effect=set_modifiers
        row=[65406,0,65406,0]
        c.get_keyboard_mapping.side_effect=lambda lo,n:[row[:]]
        c.change_keyboard_mapping.side_effect=lambda lo,rows:row.__setitem__(slice(None),rows[0])
        return c
    def test_mp08_reserve_only_inert_owned_virtual_slot(self):
        c=self.connection();m=helper(c);m.Xlib.X.MappingSuccess=0
        with patch.dict(m.os.environ,{'CHARIOX_OWNED_VIRTUAL_DISPLAY':'1'}),patch.object(m._x11_module,'open_display',return_value=c):m.prepare_owned_text_keymap()
        self.assertEqual(c.set_modifier_mapping.call_args.args[0],[[50],[0,92],[],[],[],[],[],[]])
        c.change_keyboard_mapping.assert_called_once_with(8,[[0,0,0,0]])
        c.ungrab_server.assert_called_once();c.close.assert_called_once()
    def test_mp11_held_key_refuses_before_any_keymap_change(self):
        c=self.connection();c.query_keymap.return_value=bytes([0,1])+bytes(30);m=helper(c)
        with patch.dict(m.os.environ,{'CHARIOX_OWNED_VIRTUAL_DISPLAY':'1'}),patch.object(m._x11_module,'open_display',return_value=c):
            with self.assertRaises(ValueError):m.prepare_owned_text_keymap()
        c.set_modifier_mapping.assert_not_called();c.change_keyboard_mapping.assert_not_called()
        c.ungrab_server.assert_called_once()
    def test_mp11_real_desktop_is_never_implicitly_remapped(self):
        c=self.connection();m=helper(c)
        with patch.dict(m.os.environ,{},clear=True):
            with self.assertRaises(ValueError):m.prepare_owned_text_keymap()
        c.grab_server.assert_not_called();c.change_keyboard_mapping.assert_not_called()

class PhysicalChordTests(unittest.TestCase):
    def native(self):
        c=MagicMock();c.query_keymap.return_value=bytes(32)
        c.screen.return_value.root.query_pointer.return_value.mask=0
        m=helper(c)
        names={'Down':116,'Return':36,'Control_L':37,'s':39}
        m.XK.string_to_keysym=lambda name:names.get(name,0)
        c.keysym_to_keycode.side_effect=lambda key:key
        c.keycode_to_keysym.side_effect=lambda code,level:code
        return m,c
    def test_mp11_two_non_modifier_chord_is_rejected_before_any_key(self):
        m,c=self.native()
        with patch.object(m.xtest,'fake_input') as inject, patch.object(m.time,'sleep'), patch.object(m.signal,'signal'):
            with self.assertRaises(ValueError):m.hold_input('key','Return+s',1)
        inject.assert_not_called()

    def test_mp11_non_modifier_is_readmitted_after_chord_resolution(self):
        m,c=self.native()
        checks=[]
        def admit():
            checks.append(True)
            if len(checks)==2:raise ValueError('protected leaf')
        with patch.object(m.xtest,'fake_input') as inject, patch.object(m,'focused_target',return_value={}), patch.object(m,'assert_secret_target'), patch.object(m.signal,'signal'):
            with self.assertRaises(ValueError):m.hold_input('key','CTRL+s',1,before_press=admit)
        self.assertFalse(any(call.args[1:]==(2,39) for call in inject.call_args_list))

    def test_mp11_room_key_repeat_readmits_each_press(self):
        m,c=self.native()
        checks=[]
        def admit():
            checks.append(True)
            if len(checks)==3:raise ValueError('protected leaf')
        with patch.object(m.xtest,'fake_input') as inject, patch.object(m,'focused_target',return_value={}), patch.object(m,'assert_secret_target'), patch.object(m.time,'sleep'), patch.object(m.signal,'signal'):
            with self.assertRaises(ValueError):m.key_repeat('s',2,before_press=admit)
        self.assertEqual(sum(call.args[1:]==(2,39) for call in inject.call_args_list),1)

    def test_mp11_room_cli_text_and_key_use_native_admission(self):
        m,c=self.native()
        for args,text in [([],b'private'),(['key-repeat','2'],b's')]:
            with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'room_input_guard',create=True,side_effect=ValueError('protected native focus')), patch.object(m.xtest,'fake_input') as inject:
                with self.assertRaises(ValueError):m.main(args,io.BytesIO(text))
            inject.assert_not_called()

    def test_room_navigation_keys_skip_the_text_focus_proof_but_text_and_chords_keep_it(self):
        # Chromium is never traversed by the native focus proof, so it would refuse these on every web page.
        m,c=self.native()
        for key,admitted in [('End',False),('PageDown',False),('down',False),('Page_Up',False),('ArrowLeft',False),
                             ('s',True),('Return',True),('space',True),('shift+End',True),('ctrl+a',True)]:
            with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'room_input_guard',return_value=lambda:None) as guard, patch.object(m,'key_repeat') as repeat:
                m.main(['key-repeat','1'],io.BytesIO(key.encode()))
            self.assertEqual(guard.called,admitted,key)
            self.assertEqual(repeat.call_args.kwargs['before_press'] is not None,admitted,key)
        with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'room_input_guard',side_effect=ValueError('protected native focus')), patch.object(m,'type_text') as type_text:
            with self.assertRaises(ValueError):m.main([],io.BytesIO(b'End'))
        type_text.assert_not_called()

    def test_agent_room_holds_reach_the_hold_helper(self):
        m,c=self.native()
        for args,data,call in [(['hold-key','100'],b's',('key','s',100)),(['hold-button','middle','100','10','10'],b'',('button','middle',100,10,10))]:
            with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'hold_input') as hold:
                m.main(args,io.BytesIO(data))
                hold.assert_called_once_with(*call)

    def test_agent_room_middle_click_is_admitted_like_other_clicks(self):
        m,c=self.native()
        c.screen.return_value.width_in_pixels=c.screen.return_value.height_in_pixels=100
        with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'focused_target',side_effect=m.SecretTargetChanged()), patch.object(m.xtest,'fake_input') as inject, patch.object(m.time,'sleep'), patch.object(m.signal,'signal'):
            m.main(['pointer-click','middle','1','10','10'],io.BytesIO(b''))
        self.assertEqual(sum(call.args[1:]==(4,2) for call in inject.call_args_list),1)

    def test_agent_room_paste_and_keys_reach_native_events_whoever_owns_the_clipboard(self):
        m,c=self.native()
        for key in ['CTRL+v','Control_L+Shift+V','SHIFT+Insert','Super+v']:
            with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'room_input_guard',return_value=lambda:None), patch.object(m,'key_repeat') as repeat:
                m.main(['key-repeat','2'],io.BytesIO(key.encode()))
                repeat.assert_called_once()

    def test_agent_room_double_click_needs_no_clipboard_admission(self):
        m,c=self.native()
        c.screen.return_value.width_in_pixels=c.screen.return_value.height_in_pixels=100
        with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'focused_target',side_effect=m.SecretTargetChanged()), patch.object(m.xtest,'fake_input') as inject, patch.object(m.time,'sleep'), patch.object(m.signal,'signal'):
            m.main(['pointer-click','left','2','10','10'],io.BytesIO(b''))
        self.assertEqual(sum(call.args[1:]==(4,1) for call in inject.call_args_list),2)

    def test_mp11_approved_room_vault_input_bypasses_ordinary_native_admission(self):
        m,c=self.native()
        target={'focus_window':10,'active_window':10,'geometry':[0,0,10,10],'window_geometry':[0,0,10,10]}
        with patch.dict(m.os.environ,{'CHARIOX_COMPUTER_AGENT_INPUT':'1'}), patch.object(m,'room_input_guard') as guard, patch.object(m,'type_text') as type_text:
            m.main(['secret',m.json.dumps(target)],io.BytesIO(b'approved'))
        guard.assert_not_called()
        type_text.assert_called_once_with('approved',target)

    def test_mp08_lowercase_provider_navigation_dispatches_real_base_keys(self):
        for name,code in [('down',116),('return',36)]:
            m,c=self.native()
            with patch.object(m.xtest,'fake_input') as inject, patch.object(m.time,'sleep'), patch.object(m.signal,'signal'):
                m.hold_input('key',name,1)
            self.assertEqual([x.args[1:] for x in inject.call_args_list],[(2,code),(3,code)])
    def test_mp08_shared_browser_arrow_names_dispatch_real_base_keys(self):
        for name in ('ArrowDown','ARROWDOWN'):
            m,c=self.native()
            with patch.object(m.xtest,'fake_input') as inject, patch.object(m.time,'sleep'), patch.object(m.signal,'signal'):
                m.hold_input('key',name,1)
            self.assertEqual([x.args[1:] for x in inject.call_args_list],[(2,116),(3,116)])
    def test_mp11_unknown_key_never_reports_applied(self):
        m,c=self.native()
        with patch.object(m.xtest,'fake_input') as inject, patch.object(m.signal,'signal'):
            with self.assertRaises(ValueError):m.hold_input('key','not_a_key',1)
        inject.assert_not_called();c.close.assert_called_once()
    def test_mp08_repeat_is_bounded_and_restores_cancellation_handlers(self):
        m,c=self.native()
        with patch.object(m,'hold_input') as hold, patch.object(m.time,'sleep'), patch.object(m.signal,'getsignal',return_value='handler'), patch.object(m.signal,'signal') as signals:
            m.key_repeat('CTRL+S',2)
            self.assertEqual(hold.call_count,2)
            self.assertTrue(all(x.args[1]=='handler' for x in signals.call_args_list))
            for repeat in [0,33]:
                with self.assertRaises(ValueError):m.key_repeat('s',repeat)
            self.assertEqual(hold.call_count,2)
    def test_mp11_cancelled_repeat_restores_handlers_and_never_retries(self):
        m,c=self.native()
        with patch.object(m,'hold_input',side_effect=SystemExit(143)) as hold, patch.object(m.signal,'getsignal',return_value='handler'), patch.object(m.signal,'signal') as signals:
            with self.assertRaises(SystemExit):m.key_repeat('s',2)
            hold.assert_called_once();self.assertEqual(signals.call_count,2)

if __name__ == "__main__": unittest.main()
