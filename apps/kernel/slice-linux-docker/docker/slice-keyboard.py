#!/usr/bin/env python3
"""Physical text input using the pinned Selkies XTEST keyboard implementation."""

import json
import logging
import signal
import sys
import time

logging.disable(logging.CRITICAL)

from selkies import Xlib
from selkies.Xlib import XK
from selkies.Xlib import display
from selkies.Xlib.ext import xtest
# Internal API is intentionally tied to selkies.lock.json revision
# 3f87241fcd6abc44e205b22f6596e78ef4946670. Any pin upgrade must rerun the
# physical keyboard X11 drill, including Unicode recycling and cancellation.
from selkies.input_handler import (
    _XTestKeyboard,
    character_to_layout_keysym,
    universal_text_keysym,
)


class SecretTargetChanged(Exception):
    pass


class ComputerTextKeyboard(_XTestKeyboard):
    def _find_spare_keycodes(self):
        # MP-08/MP-10/MP-11: a NoSymbol keycode is not necessarily inert.
        # Chromium can fall back to its hardware meaning (e.g. BrowserRefresh)
        # when it cannot translate an overlay Unicode keysym. X11 keycodes 8
        # and 92 have no hardware fallback in Chromium's core keycode table.
        # Recycle these slots under the existing mapping-settle guard rather
        # than lending text input media/navigation keycodes. Modifier-mapped
        # or occupied slots remain excluded by the upstream discovery.
        # Keep upstream's full _spare_set: both core and XKB lookup must
        # distrust inherited overlays on excluded media/navigation keycodes.
        # Only the allocation pool is restricted to safe hardware codes.
        return [code for code in super()._find_spare_keycodes() if code in (8, 92)]

    @staticmethod
    def _text_accelerator(keycode):
        return (keycode >= 104 or keycode in (9, 22, 23, 36, 66, 95, 96)
                or 67 <= keycode <= 78)

    def _resolve(self, keysym):
        code, modifiers, group = super()._resolve(keysym)
        if not code:
            raise ValueError("no safe physical text keycode")
        # Explicit text line breaks and tabs retain their ordinary controls.
        # A previous helper may have left Unicode on an unsafe physical code;
        # do not accept that overlay as an inherited layout binding either.
        if keysym not in (0xff0d, 0xff09) and self._text_accelerator(code):
            code = self._overlay_keycode(keysym)
            if not code:
                raise ValueError("no safe physical text keycode")
            return code, (), None
        return code, modifiers, group


def focused_target(connection):
    """MP-08: native focus identity/geometry only; never titles or field values."""
    focus = connection.get_input_focus().focus
    if not hasattr(focus, "id") or focus.id <= 1:
        raise SecretTargetChanged()
    root = connection.screen().root
    window = focus
    ancestors = []
    x = y = 0
    focus_geometry = focus.get_geometry()
    for _ in range(64):
        ancestors.append(window)
        geometry = window.get_geometry()
        x += geometry.x + geometry.border_width
        y += geometry.y + geometry.border_width
        tree = window.query_tree()
        if tree.parent.id == root.id:
            break
        if tree.parent.id <= 1 or tree.parent.id == window.id:
            raise SecretTargetChanged()
        window = tree.parent
    else:
        raise SecretTargetChanged()
    active = root.get_full_property(connection.intern_atom("_NET_ACTIVE_WINDOW"), Xlib.X.AnyPropertyType)
    active_id = int(active.value[0]) if active is not None and len(active.value) else window.id
    # Openbox reparents a client below a frame. EWMH identifies the client,
    # which must still be an ancestor of the native focused control.
    active_window = next((ancestor for ancestor in ancestors if ancestor.id == active_id), None)
    if active_window is None:
        raise SecretTargetChanged()
    window_geometry = active_window.get_geometry()
    return {
        "focus_window": focus.id,
        "active_window": active_id,
        "geometry": [x, y, focus_geometry.width, focus_geometry.height],
        "window_geometry": [window_geometry.x, window_geometry.y,
                            window_geometry.width, window_geometry.height],
    }


def assert_secret_target(connection, expected_target):
    if focused_target(connection) != expected_target:
        raise SecretTargetChanged()


def type_text(text, expected_target=None):
    connection = display.Display()
    keyboard = ComputerTextKeyboard(connection)
    lifted = []
    active_keysym = None
    try:
        fill_record = None
        if expected_target is not None:
            assert_secret_target(connection, expected_target)
            import importlib.util
            from pathlib import Path
            spec = importlib.util.spec_from_file_location('native_fill_targets', Path(__file__).with_name('native-fill-targets.py'))
            fill_targets = importlib.util.module_from_spec(spec); spec.loader.exec_module(fill_targets)
            fill_record = fill_targets.begin(expected_target['active_window'], text)
        keysyms = []
        for character in text:
            # Some layouts carry Linefeed, which Chromium accepts but GTK
            # editors ignore. Text line breaks need the universal Return
            # binding even when a layout advertises the control character.
            if character in ("\n", "\r", "\t"):
                keysym = universal_text_keysym(character)
            else:
                keysym = character_to_layout_keysym(character)
                if not keyboard.layout_carries(keysym):
                    keysym = universal_text_keysym(character)
            if keysym is None:
                raise ValueError("unsupported text character")
            keysyms.append(keysym)

        # Reuse Selkies' persistent overlay, including its bounded recycling
        # when a string contains more distinct symbols than the spare pool.
        keyboard.prebind(keysyms)
        down = connection.query_keymap()
        modifiers = {code for row in connection.get_modifier_mapping() for code in row if code}
        lifted = [code for code in modifiers if down[code // 8] & (1 << (code % 8))]
        for code in lifted:
            xtest.fake_input(connection, Xlib.X.KeyRelease, code)
        connection.sync()

        for keysym in keysyms:
            # MP-08 / MP-11: one keystroke batch under the X server grab.
            # Other display clients cannot change native focus between the
            # target check and physical input delivery. Never refocus a target.
            if expected_target is not None:
                connection.grab_server()
            try:
                if expected_target is not None:
                    assert_secret_target(connection, expected_target)
                active_keysym = keysym
                keyboard.press(keysym)
                keyboard.release(keysym)
                active_keysym = None
                connection.sync()
            finally:
                if expected_target is not None:
                    connection.ungrab_server()
                    connection.sync()
            # Pace on this process, not in the X server's request queue. Killing
            # the kernel-owned process group must stop future physical events.
            connection.sync()
            time.sleep(0.04)
    finally:
        if expected_target is not None and fill_record is not None:
            fill_targets.finish(fill_record)
        # A second termination signal must not interrupt modifier restoration.
        # The caller retains SIGKILL as its bounded last-resort cleanup.
        for signum in (signal.SIGTERM, signal.SIGINT):
            signal.signal(signum, signal.SIG_IGN)
        if active_keysym is not None:
            keyboard.release(active_keysym)
        keyboard.release_group_lock()
        for code in lifted:
            xtest.fake_input(connection, Xlib.X.KeyPress, code)
        connection.sync()
        connection.close()


def hold_input(kind, value, duration_ms, x=None, y=None):
    """MP-08/MP-10/MP-11: press/hold/release within one owned Action.

    Hold only existing base-layout keys; text overlays stay in type_text.
    Reject pre-existing native holds instead of releasing another actor's input.
    """
    if not 1 <= duration_ms <= 10000:
        raise ValueError("invalid hold duration")
    connection = display.Display()
    pressed = []
    try:
        if kind == "key":
            if not value or len(value.encode("utf-8")) > 128 or not value.isascii():
                raise ValueError("invalid chord")
            aliases = {"ctrl": "Control_L", "Control": "Control_L", "alt": "Alt_L",
                       "shift": "Shift_L", "super": "Super_L", "space": "space"}
            codes = []
            for name in value.split("+"):
                keysym = XK.string_to_keysym(aliases.get(name, name))
                code = connection.keysym_to_keycode(keysym) if keysym else 0
                # No implicit shifted symbol or Unicode hardware fallback.
                if code < 8 or connection.keycode_to_keysym(code, 0) != keysym or code in codes:
                    raise ValueError("unmapped or duplicate chord key")
                codes.append(code)
            event_type, release_type = Xlib.X.KeyPress, Xlib.X.KeyRelease
        elif kind == "button":
            codes = [{"left": 1, "middle": 2, "right": 3}[value]]
            if x is None or y is None or not (0 <= x < connection.screen().width_in_pixels
                                              and 0 <= y < connection.screen().height_in_pixels):
                raise ValueError("invalid pointer coordinates")
            event_type, release_type = Xlib.X.ButtonPress, Xlib.X.ButtonRelease
        else:
            raise ValueError("invalid hold kind")
        connection.grab_server()
        try:
            if any(connection.query_keymap()) or connection.screen().root.query_pointer().mask & 7936:
                raise ValueError("native input already held")
            if kind == "button":
                xtest.fake_input(connection, Xlib.X.MotionNotify, x=x, y=y)
            for code in codes:
                # Track before queuing so exceptional/cancelled presses release too.
                pressed.append(code)
                xtest.fake_input(connection, event_type, code)
            connection.sync()
        finally:
            connection.ungrab_server()
            connection.sync()
        time.sleep(duration_ms / 1000)
    finally:
        for signum in (signal.SIGTERM, signal.SIGINT):
            signal.signal(signum, signal.SIG_IGN)
        for code in reversed(pressed):
            xtest.fake_input(connection, release_type, code)
        connection.sync()
        connection.close()


def reset_input():
    connection = display.Display()
    try:
        down = connection.query_keymap()
        for code in range(8, 256):
            if down[code // 8] & (1 << (code % 8)):
                xtest.fake_input(connection, Xlib.X.KeyRelease, code)
        for button in range(1, 6):
            xtest.fake_input(connection, Xlib.X.ButtonRelease, button)
        connection.sync()
    finally:
        connection.close()


if __name__ == "__main__":
    def terminate(signum, _frame):
        raise SystemExit(128 + signum)

    for signum in (signal.SIGTERM, signal.SIGINT):
        signal.signal(signum, terminate)
    try:
        if len(sys.argv) == 3 and sys.argv[1] == "hold-key":
            hold_input("key", sys.stdin.buffer.read(129).decode("ascii", errors="strict"), int(sys.argv[2]))
        elif len(sys.argv) == 6 and sys.argv[1] == "hold-button":
            hold_input("button", sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]))
        elif sys.argv[1:] == ["reset"]:
            reset_input()
        elif sys.argv[1:] == ["secret-target"]:
            connection = display.Display()
            try:
                print(json.dumps(focused_target(connection), separators=(",", ":")))
            finally:
                connection.close()
        elif len(sys.argv) == 3 and sys.argv[1] == "secret":
            expected_target = json.loads(sys.argv[2])
            if not isinstance(expected_target, dict) or set(expected_target) != {
                "focus_window", "active_window", "geometry", "window_geometry"
            }:
                raise ValueError("invalid secret target")
            type_text(sys.stdin.buffer.read().decode("utf-8", errors="strict"), expected_target)
        elif not sys.argv[1:]:
            type_text(sys.stdin.buffer.read().decode("utf-8", errors="strict"))
        else:
            raise ValueError("unsupported keyboard operation")
    except SecretTargetChanged:
        print("computer credential input aborted: focused control or window changed", file=sys.stderr)
        sys.exit(2)
    except Exception:
        # Neither typed text nor upstream exceptions belong in helper output.
        print("physical keyboard text input failed", file=sys.stderr)
        sys.exit(1)
