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
    def test_occupied_or_modifier_slots_fail_before_injection(self):
        self.mapping[8]=self.mapping[92]=[ord('b')]*2
        with patch.object(module.xtest,'fake_input') as inject:
            with self.assertRaises(ValueError):self.keyboard.press(0x01004e16)
            inject.assert_not_called()
if __name__=='__main__':unittest.main()
