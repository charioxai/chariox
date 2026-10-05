"""MP-08/MP-10/MP-11: inherited overlays through the pinned keyboard API.

Run with /opt/chariox-selkies/bin/python in the dependency fixture image.
Lookup and press/release use upstream code; only the display and XTEST are fake.
"""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import Mock, patch


spec = importlib.util.spec_from_file_location("computer_keyboard", Path(__file__).with_name("slice-keyboard.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class InheritedOverlayTests(unittest.TestCase):
    def setUp(self):
        self.keysym = 0x01004E16
        self.unsafe = 181  # Chromium's hardware BrowserRefresh fallback.
        self.display = Mock()
        self.display.display.info.min_keycode = 8
        self.display.display.info.max_keycode = 255
        self.display.get_modifier_mapping.return_value = [[50]]
        self.mapping = [[0, 0] for _ in range(248)]
        self.mapping[38 - 8] = [ord("a"), ord("A")]
        self.mapping[self.unsafe - 8] = [self.keysym] * 2
        self.display.get_keyboard_mapping.side_effect = lambda lo, count: self.mapping[lo - 8:lo - 8 + count]
        self.display.keysym_to_keycode.side_effect = lambda ks: {self.keysym: self.unsafe, ord("a"): 38}.get(ks, 0)
        self.xkb = Mock()
        self.xkb.locate.side_effect = lambda ks: {self.keysym: (self.unsafe, 0, 0), ord("a"): (38, 0, 0)}.get(ks)
        with patch("selkies.input_handler.open_xkb_link", return_value=self.xkb):
            self.keyboard = module.ComputerTextKeyboard(self.display)

    def test_allocation_filters_without_losing_inherited_overlay_distrust(self):
        pool = self.keyboard._find_spare_keycodes()
        self.assertEqual(pool, [8, 92])
        self.assertIn(self.unsafe, self.keyboard._spare_set)
        self.assertNotIn(38, self.keyboard._spare_set)
        self.assertNotIn(50, self.keyboard._spare_set)

    def test_core_and_xkb_lookup_reject_inherited_unsafe_overlay(self):
        self.assertEqual(self.keyboard._layout_keycode(self.keysym), 0)
        self.assertIsNone(self.keyboard._placement(self.keysym))
        self.assertFalse(self.keyboard.layout_carries(self.keysym))
        self.assertEqual(self.keyboard._layout_keycode(ord("a")), 38)
        self.assertEqual(self.keyboard._placement(ord("a")), (38, 0, 0))

    def test_inherited_overlay_press_and_release_use_owned_safe_binding(self):
        for xkb in (self.xkb, None):
            with self.subTest(xkb=xkb is not None):
                with patch("selkies.input_handler.open_xkb_link", return_value=xkb):
                    self.keyboard = module.ComputerTextKeyboard(self.display)
                with patch.object(module.xtest, "fake_input") as inject, patch.object(self.keyboard, "_settle"):
                    self.keyboard.press(self.keysym)
                    self.keyboard.release(self.keysym)
                self.assertEqual([c.args[1:] for c in inject.call_args_list],
                                 [(module.Xlib.X.KeyPress, 8), (module.Xlib.X.KeyRelease, 8)])
                self.assertEqual(self.keyboard._layout_keycode(self.keysym), 8)
                self.assertEqual(self.keyboard._pressed_kc, {})

    def test_prebind_reclaims_inherited_overlay_into_safe_pool(self):
        self.assertTrue(self.keyboard.prebind([self.keysym]))
        self.display.change_keyboard_mapping.assert_called_once_with(8, [[self.keysym] * 2])
        self.assertEqual(self.keyboard._layout_keycode(self.keysym), 8)
        self.assertEqual(self.keyboard._placement(self.keysym), (8, 0, 0))

    def test_no_safe_pool_fails_before_pressing_inherited_unsafe_code(self):
        self.mapping[0] = self.mapping[92 - 8] = [ord("b"), ord("B")]
        with patch.object(module.xtest, "fake_input") as inject:
            with self.assertRaises(ValueError):
                self.keyboard.press(self.keysym)
        inject.assert_not_called()

    def test_occupied_and_modifier_codes_stay_excluded(self):
        self.mapping[0] = [ord("b"), ord("B")]
        self.display.get_modifier_mapping.return_value = [[50, 92]]
        self.assertEqual(self.keyboard._find_spare_keycodes(), [])


if __name__ == "__main__":
    unittest.main()
