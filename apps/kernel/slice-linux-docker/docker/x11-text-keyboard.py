"""MP-08 / MP-11: portable XTEST text backend for the shared keyboard helper.

Host desktop keymap is US. Overlay only inert physical codes 8/92; never reuse
media/accelerator codes or a modifier. Slice retains its pinned layout backend.
"""
import time
from Xlib import X, XK
from Xlib.ext import xtest


def universal_text_keysym(character):
    if character in ('\n', '\r'): return 0xff0d
    if character == '\t': return 0xff09
    code = ord(character)
    if code < 32 or 0x7f <= code < 0xa0: return None
    return code if code < 256 else 0x01000000 | code


character_to_layout_keysym = universal_text_keysym


class _XTestKeyboard:
    def __init__(self, connection):
        self.connection = connection
        self.overlay = {}
        self._pressed_kc = {}
        self.modifiers = []
        self._spare_set = frozenset()
        self.saved = {}
        self._find_spare_keycodes()

    def _find_spare_keycodes(self):
        modifiers = {code for row in self.connection.get_modifier_mapping() for code in row if code}
        codes = [code for code in (8, 92) if code not in modifiers and
                 (code in self.saved or all(not value or value >= 0x01000000 for value in self.connection.get_keyboard_mapping(code, 1)[0]))]
        self._spare_set = frozenset(codes)
        return codes

    def _placement(self, keysym):
        # Never trust inherited Unicode mappings (including hardware fallback codes).
        if keysym >= 0x01000000: return None
        for code, row in enumerate(self.connection.get_keyboard_mapping(8, 248), 8):
            if code in self._spare_set or code >= 104: continue
            for index, value in enumerate(row[:2]):
                if value == keysym: return code, index, 0
        return None

    def layout_carries(self, keysym): return self._placement(keysym) is not None
    def prebind(self, keysyms): return False  # Bounded recycling at each stroke.
    def _settle(self):
        self.connection.sync()
        time.sleep(0.04)

    def _overlay_keycode(self, keysym):
        if keysym in self.overlay: return self.overlay[keysym]
        pool = self._find_spare_keycodes()
        if not pool: raise ValueError('no safe text slot')
        code = pool[0]
        if code not in self.saved:
            self.saved[code] = list(self.connection.get_keyboard_mapping(code, 1)[0])
        self.overlay = {symbol: old for symbol, old in self.overlay.items() if old != code}
        self.connection.change_keyboard_mapping(code, [[keysym] * len(self.saved[code])])
        self.overlay[keysym] = code
        self._settle()
        return code

    def _resolve(self, keysym):
        placement = self._placement(keysym)
        if placement:
            code, shift, _ = placement
            return code, (self.connection.keysym_to_keycode(XK.string_to_keysym('Shift_L')),) if shift else (), None
        return self._overlay_keycode(keysym), (), None

    def press(self, keysym):
        code, modifiers, _ = self._resolve(keysym)
        self._pressed_kc[keysym] = code
        self.modifiers = list(modifiers)
        for modifier in modifiers: xtest.fake_input(self.connection, X.KeyPress, modifier)
        xtest.fake_input(self.connection, X.KeyPress, code)

    def release(self, keysym):
        code = self._pressed_kc.pop(keysym, None)
        if code: xtest.fake_input(self.connection, X.KeyRelease, code)
        for modifier in reversed(self.modifiers): xtest.fake_input(self.connection, X.KeyRelease, modifier)
        self.modifiers = []

    def release_group_lock(self):
        for code, row in self.saved.items(): self.connection.change_keyboard_mapping(code, [row])
        self.connection.sync()
