"""Unicode text must not become Chromium's physical browser accelerators."""
import importlib.util
from pathlib import Path
import sys
from types import SimpleNamespace, ModuleType
import unittest
from unittest.mock import patch


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
    xlib.X = SimpleNamespace(KeyRelease=3, KeyPress=2)
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


if __name__ == "__main__": unittest.main()
