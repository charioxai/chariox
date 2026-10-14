"""MP-08 / MP-11: portable host Unicode overlay safety, fail-first recycling."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import Mock, patch
spec=importlib.util.spec_from_file_location('keyboard',Path(__file__).with_name('x11-text-keyboard.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class TextTests(unittest.TestCase):
    def setUp(self):
        self.mapping={code:[0,0] for code in range(8,256)}
        self.mapping[181]=[0x01004e16]*2
        self.mapping[38]=[ord('a'),ord('A')]
        self.connection=Mock()
        self.connection.get_modifier_mapping.return_value=[[50]]
        self.connection.get_keyboard_mapping.side_effect=lambda lo,n:[self.mapping[c] for c in range(lo,lo+n)]
        self.connection.change_keyboard_mapping.side_effect=lambda code,rows:self.mapping.update({code:rows[0]})
        self.fence=Mock()
        self.factory=patch.object(module._fence, 'KeymapFence', return_value=self.fence)
        self.factory.start(); self.addCleanup(self.factory.stop)
        self.keyboard=module._XTestKeyboard(self.connection)
    def test_inherited_hardware_overlay_is_not_text(self):
        self.assertIsNone(self.keyboard._placement(0x01004e16))
        with patch.object(self.keyboard,'_settle'),patch.object(module.xtest,'fake_input') as inject:
            self.keyboard.press(0x01004e16);self.keyboard.release(0x01004e16)
            self.assertEqual(inject.call_args_list[0].args[2],8)
    def test_owned_latin_slot_recycles_and_restores_original_mapping(self):
        with patch.object(self.keyboard,'_settle'),patch.object(module.xtest,'fake_input'):
            for character in 'üß世界😀':
                symbol=module.universal_text_keysym(character)
                self.keyboard.press(symbol);self.keyboard.release(symbol)
            self.keyboard.release_group_lock()
        self.assertEqual(self.mapping[8],[0,0])
    def test_unicode_waits_for_the_client_keymap_before_each_physical_stroke(self):
        # MP-08/MP-10/MP-11: X sync acknowledges the server, not the client
        # keymap refresh. Model Chrome's delayed map reply under display load.
        now = 0
        client_mapping = {}
        updates = []
        delivered = []
        def mapping(code, rows):
            self.mapping[code] = rows[0]
            updates.append((now + 0.06, code, rows[0][0]))
        def sleep(seconds):
            nonlocal now
            now += seconds
            while updates and updates[0][0] <= now:
                _, code, symbol = updates.pop(0)
                client_mapping[code] = symbol
        def event(_connection, kind, code):
            if kind == module.X.KeyPress:
                delivered.append(client_mapping.get(code, 0))
        symbols = [module.universal_text_keysym(c) for c in '漢字🧪' * 4]
        self.connection.change_keyboard_mapping.side_effect = mapping
        self.fence.wait.side_effect = lambda: sleep(max(0, updates[-1][0] - now))
        with patch.object(module.xtest, 'fake_input', side_effect=event):
            for symbol in symbols:
                self.keyboard.press(symbol)
                self.keyboard.release(symbol)
                sleep(0.04)
        self.assertEqual(delivered, symbols)
        self.assertEqual(self.fence.wait.call_count, len(symbols))

    def test_occupied_or_modifier_slots_fail_before_injection(self):
        self.mapping[8]=self.mapping[92]=[ord('b')]*2
        with patch.object(module.xtest,'fake_input') as inject:
            with self.assertRaises(ValueError):self.keyboard.press(0x01004e16)
            inject.assert_not_called()

    def test_application_consumes_each_stroke_before_its_mapping_is_recycled(self):
        pending, delivered = [], []
        self.fence.delivered.side_effect = lambda: delivered.extend(
            self.mapping[code][0] for code in pending)
        symbols = [module.universal_text_keysym(c) for c in '漢字🧪' * 3]
        def event(_connection, kind, code):
            if kind == module.X.KeyPress: pending.append(code)
        with patch.object(self.keyboard, '_settle'), patch.object(module.xtest, 'fake_input', side_effect=event):
            for symbol in symbols:
                self.keyboard.press(symbol)
                self.keyboard.release(symbol)
                self.keyboard.settle_text()
                pending.clear()
        self.assertEqual(delivered, symbols)
if __name__=='__main__':unittest.main()
