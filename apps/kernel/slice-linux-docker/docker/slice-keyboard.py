#!/usr/bin/env python3
"""Physical text input using the pinned Selkies XTEST keyboard implementation."""
# MP-08/MP-11: do not connect an owned display number to a host filesystem socket.
import importlib.util as _x11_import
from pathlib import Path as _X11Path
_x11_spec=_x11_import.spec_from_file_location('native_x11',_X11Path(__file__).with_name('native-x11.py'))
_x11_module=_x11_import.module_from_spec(_x11_spec);_x11_spec.loader.exec_module(_x11_module)


import json
import logging
import os
import signal
import sys
import time

logging.disable(logging.CRITICAL)

try:
    from selkies import Xlib
    from selkies.Xlib import XK, display
    from selkies.Xlib.ext import xtest
    from selkies.input_handler import (
        _XTestKeyboard, character_to_layout_keysym, universal_text_keysym,
    )
except ModuleNotFoundError as error:
    if not error.name.startswith("selkies"):
        raise
    import Xlib
    from Xlib import XK, display
    from Xlib.ext import xtest
    import importlib.util
    from pathlib import Path
    spec = importlib.util.spec_from_file_location("x11_text_keyboard", Path(__file__).with_name("x11-text-keyboard.py"))
    backend = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(backend)
    _XTestKeyboard = backend._XTestKeyboard
    character_to_layout_keysym = backend.character_to_layout_keysym
    universal_text_keysym = backend.universal_text_keysym


def prepare_owned_text_keymap():
    """MP-08 / MP-11: reserve code8 only during owned virtual-display boot."""
    if os.environ.get('CHARIOX_OWNED_VIRTUAL_DISPLAY') != '1':
        raise ValueError('owned virtual display required')
    connection=_x11_module.open_display(display)
    connection.grab_server()
    original_modifiers=None
    original_row=None
    try:
        if any(connection.query_keymap()):
            raise ValueError('held physical keys prevent keymap preparation')
        original_modifiers=[list(row) for row in connection.get_modifier_mapping()]
        original_row=list(connection.get_keyboard_mapping(8,1)[0])
        modifiers=[[0 if code==8 else code for code in row] for row in original_modifiers]
        if connection.set_modifier_mapping(modifiers) != Xlib.X.MappingSuccess:
            raise ValueError('virtual keymap modifier preparation refused')
        connection.change_keyboard_mapping(8,[[0]*len(original_row)])
        connection.sync()
        if any(connection.get_keyboard_mapping(8,1)[0]) or any(8 in row for row in connection.get_modifier_mapping()):
            raise ValueError('virtual text slot unavailable')
    except Exception:
        if original_row is not None:
            connection.change_keyboard_mapping(8,[original_row])
            connection.set_modifier_mapping(original_modifiers)
            connection.sync()
        raise
    finally:
        connection.ungrab_server()
        connection.sync()
        connection.close()


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


def type_text(text, expected_target=None, before_press=None, *, pace_seconds=0.04):
    connection = _x11_module.open_display(display)
    keyboard = ComputerTextKeyboard(connection)
    lifted = []
    active_keysym = None
    try:
        vault_input = expected_target is not None
        fill_record = None
        if before_press is not None:
            expected_target=focused_target(connection)
            before_press()
        if expected_target is not None:
            assert_secret_target(connection, expected_target)
            import importlib.util
            from pathlib import Path
            spec = importlib.util.spec_from_file_location('native_fill_targets', Path(__file__).with_name('native-fill-targets.py'))
            fill_targets = importlib.util.module_from_spec(spec); spec.loader.exec_module(fill_targets)
            if vault_input: fill_record = fill_targets.begin(expected_target['active_window'], text, connection)
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
            # MP-11: inspect native leaf protection outside the server grab;
            # fence its X focus inside the grab before each physical batch.
            if before_press is not None:before_press()
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
            if pace_seconds:
                time.sleep(pace_seconds)
    finally:
        if expected_target is not None and fill_record is not None:
            fill_targets.finish(fill_record, text)
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


def hold_input(kind, value, duration_ms, x=None, y=None, before_press=None):
    """MP-08/MP-10/MP-11: press/hold/release within one owned Action.

    Hold only existing base-layout keys; text overlays stay in type_text.
    Reject pre-existing native holds instead of releasing another actor's input.
    """
    if not 1 <= duration_ms <= 10000:
        raise ValueError("invalid hold duration")
    connection = _x11_module.open_display(display)
    pressed = []
    try:
        expected_target=None
        if before_press is not None:
            if duration_ms!=1:raise ValueError('agent native repeats unavailable')
            # MP-11: keys bind to the focused control; clicks choose their own target.
            if kind=="key":expected_target=focused_target(connection)
            before_press()
        if kind == "key":
            if not value or len(value.encode("utf-8")) > 128 or not value.isascii():
                raise ValueError("invalid chord")
            # MP-08: chords name physical base keys; provider casing is not Shift.
            aliases = {"ctrl": "Control_L", "control": "Control_L", "alt": "Alt_L",
                       "shift": "Shift_L", "super": "Super_L", "meta": "Super_L",
                       "enter": "Return", "return": "Return", "esc": "Escape",
                       "escape": "Escape", "tab": "Tab", "space": "space",
                       "backspace": "BackSpace", "delete": "Delete", "left": "Left",
                       "right": "Right", "up": "Up", "down": "Down", "home": "Home",
                       # Shared Browser surface key names.
                       "arrowleft": "Left", "arrowright": "Right", "arrowup": "Up",
                       "arrowdown": "Down",
                       "end": "End", "pageup": "Prior", "pagedown": "Next"}
            codes = []
            non_modifiers = set()
            modifiers = {'Control_L', 'Control_R', 'Alt_L', 'Alt_R', 'Shift_L', 'Shift_R',
                         'Super_L', 'Super_R', 'Meta_L', 'Meta_R', 'Hyper_L', 'Hyper_R'}
            for name in value.split("+"):
                base = aliases.get(name.lower(), name)
                if len(base) == 1 and base.isalpha():
                    base = base.lower()
                elif base.lower().startswith("f") and base[1:].isdigit():
                    base = base.upper()
                keysym = XK.string_to_keysym(base)
                code = connection.keysym_to_keycode(keysym) if keysym else 0
                # No implicit shifted symbol or Unicode hardware fallback.
                if code < 8 or connection.keycode_to_keysym(code, 0) != keysym or code in codes:
                    raise ValueError("unmapped or duplicate chord key")
                codes.append(code)
                if base not in modifiers:
                    non_modifiers.add(code)
            if len(non_modifiers) > 1:
                raise ValueError('chord permits at most one non-modifier key')
            # Check the live leaf after resolution, before sending modifiers.
            if non_modifiers and before_press is not None:
                before_press()
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
            if expected_target is not None:assert_secret_target(connection,expected_target)
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


def key_repeat(value, repeat, before_press=None):
    """MP-08/MP-11: strict shared native chords, bounded repeat, cancellable."""
    if not 1 <= repeat <= 32:
        raise ValueError("invalid key repeat")
    for index in range(repeat):
        handlers = {number: signal.getsignal(number) for number in (signal.SIGTERM, signal.SIGINT)}
        try:
            hold_input("key", value, 1, before_press=before_press)
        finally:
            # hold_input shields key-up cleanup; restore cancellation between chords.
            for number, handler in handlers.items():
                signal.signal(number, handler)
        if index + 1 < repeat:
            time.sleep(0.04)


def reset_input():
    connection = _x11_module.open_display(display)
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


def room_input_guard():
    import importlib.util
    from pathlib import Path
    spec = importlib.util.spec_from_file_location('room_native_protection', Path(__file__).with_name('room-native-protection.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.input_guard()


def room_clipboard_guard():
    import importlib.util
    from pathlib import Path
    def load(name):
        spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(name+'.py'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
    room=load('room-native-protection')
    try:return load('native-clipboard').input_admission(room.binding(),room.accessibility())
    except Exception as error:raise room.RoomInputDenied('native Room clipboard unavailable') from error


def room_agent_guard():
    """MP-11: every agent Room key/text press may reach a Paste control."""
    admit_focus=room_input_guard()
    admit_clipboard=room_clipboard_guard()
    def guard():
        admit_focus()
        admit_clipboard()
    return guard


def main(args, stream):
    agent = os.environ.get('CHARIOX_COMPUTER_AGENT_INPUT') == '1'
    if agent and args and args[0] in ('hold-key','hold-button'):
        import importlib.util
        from pathlib import Path
        spec=importlib.util.spec_from_file_location('room_native_protection',Path(__file__).with_name('room-native-protection.py'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        raise module.RoomInputDenied('native Room holds require human or approved Vault input')
    guard = room_agent_guard() if agent and (not args or args[0] == 'key-repeat') else None
    if args == ['prepare-owned-keymap'] and not agent:
        prepare_owned_text_keymap()
    elif len(args) == 2 and args[0] == 'key-repeat':
        key_repeat(stream.read(129).decode('ascii', errors='strict'), int(args[1]), before_press=guard)
    elif len(args) == 2 and args[0] == 'hold-key':
        hold_input('key', stream.read(129).decode('ascii', errors='strict'), int(args[1]))
    elif len(args) == 5 and args[0] == 'hold-button':
        hold_input('button', args[1], int(args[2]), int(args[3]), int(args[4]))
    elif len(args) == 5 and args[0] == 'pointer-click' and agent:
        # MP-11: every native click can activate a Paste control. Admit the
        # clipboard owner once per click action; each press only fences it.
        if args[1] not in ('left','right') or args[2] not in ('1','2'):
            raise ValueError('invalid admitted pointer click')
        admit_clipboard=room_clipboard_guard()
        for index in range(int(args[2])):
            handlers={number:signal.getsignal(number) for number in (signal.SIGTERM,signal.SIGINT)}
            try:hold_input('button',args[1],1,int(args[3]),int(args[4]),before_press=admit_clipboard)
            finally:
                for number,handler in handlers.items():signal.signal(number,handler)
            if index+1<int(args[2]):time.sleep(0.08)
    elif args == ['reset'] and not agent:
        reset_input()
    elif args == ['secret-target']:
        connection = _x11_module.open_display(display)
        try:
            print(json.dumps(focused_target(connection), separators=(',', ':')))
        finally:
            connection.close()
    elif len(args) == 2 and args[0] == 'secret':
        expected_target = json.loads(args[1])
        if not isinstance(expected_target, dict) or set(expected_target) != {
            'focus_window', 'active_window', 'geometry', 'window_geometry'
        }:
            raise ValueError('invalid secret target')
        type_text(stream.read().decode('utf-8', errors='strict'), expected_target)
    elif not args:
        type_text(stream.read().decode('utf-8', errors='strict'), before_press=guard)
    else:
        raise ValueError('unsupported keyboard operation')


if __name__ == '__main__':
    def terminate(signum, _frame):
        raise SystemExit(128 + signum)

    for signum in (signal.SIGTERM, signal.SIGINT):
        signal.signal(signum, terminate)
    try:
        main(sys.argv[1:], sys.stdin.buffer)
    except SecretTargetChanged:
        print('computer credential input aborted: focused control or window changed', file=sys.stderr)
        sys.exit(2)
    except Exception as error:
        # MP-11: only native admission failures use the typed focus refusal.
        if type(error).__name__ in ('NativeInputDenied', 'RoomInputDenied'):
            print('user_domain_sensitive_requires_focus', file=sys.stderr)
        else:
            print('physical keyboard text input failed', file=sys.stderr)
        sys.exit(1)
