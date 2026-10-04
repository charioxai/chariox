"""MP-08 / MP-11: target checks and keystroke batching without real secrets."""
import importlib.util
import pathlib
import sys
import types
import unittest
from unittest.mock import Mock, patch


def load_keyboard():
    # Keep the test independent of a Selkies install. The live X11 drill covers
    # the pinned upstream and native focus/geometry observations separately.
    xlib = types.ModuleType("selkies.Xlib")
    xlib.X = types.SimpleNamespace(KeyRelease=3, KeyPress=2, AnyPropertyType=0)
    modules = {name: types.ModuleType(name) for name in (
        "selkies", "selkies.Xlib.display", "selkies.Xlib.ext", "selkies.Xlib.ext.xtest",
        "selkies.input_handler")}
    modules["selkies.Xlib"] = xlib
    modules["selkies"].Xlib = xlib
    xlib.display = modules["selkies.Xlib.display"]
    modules["selkies.Xlib.ext"].xtest = modules["selkies.Xlib.ext.xtest"]
    modules["selkies.input_handler"]._XTestKeyboard = object
    modules["selkies.input_handler"].character_to_layout_keysym = ord
    modules["selkies.input_handler"].universal_text_keysym = ord
    spec = importlib.util.spec_from_file_location("secret_keyboard", pathlib.Path(__file__).with_name("slice-keyboard.py"))
    keyboard = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, modules):
        spec.loader.exec_module(keyboard)
    return keyboard


class SecretTargetTests(unittest.TestCase):
    def setUp(self):
        self.module = load_keyboard()
        self.connection = Mock()
        self.connection.query_keymap.return_value = bytes(32)
        self.connection.get_modifier_mapping.return_value = []
        self.module.display.Display = Mock(return_value=self.connection)
        self.keyboard = Mock()
        self.module._BrowserSafeTextKeyboard = Mock(return_value=self.keyboard)
        self.target = {"focus_window": 101, "active_window": 100,
                       "geometry": [20, 30, 200, 40], "window_geometry": [0, 0, 800, 600]}

    def type_with_targets(self, targets):
        with patch.object(self.module, "focused_target", side_effect=targets), patch.object(self.module.time, "sleep"):
            self.module.type_text("abc", self.target)

    def test_native_target_recognizes_a_reparented_active_client(self):
        root = Mock(id=10)
        frame = Mock(id=100)
        client = Mock(id=101)
        focus = Mock(id=102)
        self.connection.screen.return_value.root = root
        self.connection.get_input_focus.return_value.focus = focus
        self.connection.get_full_property = Mock()
        root.get_full_property.return_value.value = [client.id]
        focus.get_geometry.return_value = types.SimpleNamespace(x=5, y=6, border_width=0, width=150, height=40)
        client.get_geometry.return_value = types.SimpleNamespace(x=1, y=20, border_width=0, width=500, height=300)
        frame.get_geometry.return_value = types.SimpleNamespace(x=10, y=30, border_width=0, width=502, height=322)
        focus.query_tree.return_value.parent = client
        client.query_tree.return_value.parent = frame
        frame.query_tree.return_value.parent = root
        self.assertEqual(self.module.focused_target(self.connection), {
            "focus_window": focus.id, "active_window": client.id,
            "geometry": [16, 56, 150, 40], "window_geometry": [1, 20, 500, 300]})
        root.get_full_property.return_value.value = [200]
        with self.assertRaises(self.module.SecretTargetChanged):
            self.module.focused_target(self.connection)

    def test_focus_change_after_approval_aborts_before_typing(self):
        changed = dict(self.target, focus_window=102)
        with self.assertRaises(self.module.SecretTargetChanged):
            self.type_with_targets([changed])
        self.keyboard.press.assert_not_called()
        self.connection.close.assert_called_once()

    def test_window_change_mid_insertion_aborts_next_batch(self):
        changed = dict(self.target, active_window=200)
        with self.assertRaises(self.module.SecretTargetChanged):
            self.type_with_targets([self.target, self.target, changed])
        self.assertEqual(self.keyboard.press.call_count, 1)
        self.assertEqual(self.connection.grab_server.call_count, 2)
        self.assertEqual(self.connection.ungrab_server.call_count, 2)
        self.connection.close.assert_called_once()

    def test_geometry_change_aborts_next_batch(self):
        changed = dict(self.target, geometry=[21, 30, 200, 40])
        with self.assertRaises(self.module.SecretTargetChanged):
            self.type_with_targets([self.target, changed])
        self.keyboard.press.assert_not_called()
        self.connection.ungrab_server.assert_called_once()

    def test_stable_target_types_every_batch_with_server_grab(self):
        events = []
        self.connection.grab_server.side_effect = lambda: events.append("grab")
        self.connection.ungrab_server.side_effect = lambda: events.append("ungrab")
        self.keyboard.press.side_effect = lambda *_: events.append("press")
        self.keyboard.release.side_effect = lambda *_: events.append("release")
        self.type_with_targets([self.target] * 4)
        self.assertEqual(events, ["grab", "press", "release", "ungrab"] * 3)
        self.assertEqual(self.keyboard.press.call_count, 3)


if __name__ == "__main__":
    unittest.main()
