#!/usr/bin/env python3
"""Physical text input using the pinned Selkies XTEST keyboard implementation."""

import json
import logging
import signal
import sys
import time

logging.disable(logging.CRITICAL)

from selkies import Xlib
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


class _BrowserSafeTextKeyboard(_XTestKeyboard):
    """Keep text overlays away from Chromium's physical accelerators.

    Xorg can report browser/media and function keycodes as NoSymbol. Chromium
    still handles their physical code: an overlay character on BrowserRefresh
    reloads the page rather than typing it. Keep the main text-key range only,
    excluding controls and F1..F12. Selkies retains its bounded recycling.
    """

    @staticmethod
    def _text_accelerator(keycode):
        return (keycode >= 104 or keycode in (9, 22, 23, 36, 66, 95, 96)
                or 67 <= keycode <= 78)

    def _find_spare_keycodes(self):
        slots = [code for code in super()._find_spare_keycodes()
                 if not self._text_accelerator(code)]
        self._spare_set = frozenset(slots)
        return slots

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


class SecretTargetChanged(Exception):
    pass


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
    keyboard = _BrowserSafeTextKeyboard(connection)
    lifted = []
    active_keysym = None
    try:
        if expected_target is not None:
            assert_secret_target(connection, expected_target)
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
        if sys.argv[1:] == ["reset"]:
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
