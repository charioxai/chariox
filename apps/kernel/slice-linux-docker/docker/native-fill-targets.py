"""MP-08/MP-11: private, generation-scoped AT-SPI Vault fill targets.

Records only object/window identities, value fingerprints and lengths. Never
searches window text for echoes. XDG_RUNTIME_DIR belongs to this desktop epoch.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import time


def store_path():
    root = os.environ.get('XDG_RUNTIME_DIR')
    if not root:
        home = os.environ.get('CHARIOX_HOME')
        if not home: return None
        root = str(Path(home) / 'state' / 'native-fill-targets')
        Path(root).mkdir(mode=0o700, parents=True, exist_ok=True)
    display = hashlib.sha256(os.environ.get('DISPLAY','').encode()).hexdigest()[:16]
    return Path(root) / ('chariox-vault-fill-targets-' + display + '.json')


def read():
    path = store_path()
    if path is None or not path.exists(): return []
    return json.loads(path.read_text())


def write(targets):
    path = store_path()
    if path is None: return
    temporary = path.with_suffix('.new')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as output: json.dump(targets, output)
    os.replace(temporary, path)


def identity(node):
    pid = node.get_process_id()
    stat = Path('/proc/' + str(pid) + '/stat').read_text()
    return {'pid': pid, 'started': stat[stat.rfind(')')+2:].split()[19], 'path': node.path}


def field_value(node):
    text = node.queryText()
    return text.getText(0, text.characterCount)


def update(change):
    path = store_path()
    if path is None: return
    fd = os.open(path.with_suffix('.lock'), os.O_CREAT | os.O_RDWR, 0o600)
    with os.fdopen(fd, 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        write(change(read()))


def open_display(display):
    # Reuse PR5's owned-display connector; local slices use the normal X server.
    helper = Path(__file__).with_name('native-x11.py')
    if helper.exists():
        import importlib.util
        spec = importlib.util.spec_from_file_location('native_x11', helper)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module.open_display(display)
    return display.Display()


def begin(expected_window, value, connection=None):
    # The kernel already bound X11 focus; match the owning native application.
    try:
        import pyatspi
        try:
            from Xlib import X, display
        except ModuleNotFoundError:
            from selkies.Xlib import X, display
        owned_connection = connection is None
        connection = connection or open_display(display)
        try:
            window = connection.create_resource_object('window', expected_window)
            pid = window.get_full_property(connection.intern_atom('_NET_WM_PID'), X.AnyPropertyType)
            pid = int(pid.value[0]) if pid is not None and len(pid.value) else None
        finally:
            if owned_connection: connection.close()
        desktop = pyatspi.Registry.getDesktop(0)
        pending = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
        if desktop.childCount > 64: raise ValueError('Vault fill field unavailable')
        candidates = []
        visited = 0
        complete = True
        while pending and visited < 8192:
            node = pending.pop(); visited += 1
            if not node or node.get_process_id() != pid: continue
            state = node.getState()
            if state.contains(pyatspi.STATE_FOCUSED) and state.contains(pyatspi.STATE_EDITABLE): candidates.append(node)
            count = node.childCount
            if count > 8192-visited: complete = False
            pending.extend(node.getChildAtIndex(i) for i in range(min(count, 8192-visited)))
        if not complete or pending or len(candidates) != 1: raise ValueError('Vault fill field unavailable')
        node = candidates[0]
        target = {**identity(node), 'window': expected_window, 'pending': True,
                  'value_hash': hashlib.sha256(value.encode()).hexdigest(), 'length': len(value), 'registration': time.monotonic_ns()}
        text = node.queryText()
        offset, selected = text.caretOffset, 0
        if text.getNSelections():
            start, end = text.getSelection(0)
            offset = min(start, end)
            selected = abs(end - start)
        update(lambda targets: [t for t in targets if (t['pid'],t['path']) != (target['pid'],target['path'])] + [target])
        return node, target, (text.characterCount, offset, selected)
    except Exception: raise ValueError('Vault fill field unavailable') from None


def bind_delivery(node, target):
    """MP-11: a deadline or a partial value is not input-delivery evidence."""
    import pyatspi
    insertion = target.get('insertion')
    if insertion is None: return False
    if node.queryText().characterCount != target['length']: return False
    if node.getRole() == pyatspi.ROLE_PASSWORD_TEXT:
        # Dots cannot establish same-length selected replacement. Keep it
        # pending until a reveal can verify the insertion fingerprint.
        if target['length'] == target['initial_length']: return False
    else:
        value = field_value(node)
        offset, length = insertion['offset'], insertion['length']
        if len(value) != target['length'] or hashlib.sha256(value[offset:offset+length].encode()).hexdigest() != insertion['hash']: return False
        target['value_hash'] = hashlib.sha256(value.encode()).hexdigest()
        del target['insertion']
    target['pending'] = False
    del target['initial_length']
    update(lambda targets: [target if item['registration'] == target['registration'] else item for item in targets])
    return True


def finish(record, inserted):
    if record is None: return
    node, target, (initial_length, offset, selected) = record
    try:
        target['length'] = initial_length - selected + len(inserted)
        target['initial_length'] = initial_length
        target['value_hash'] = None
        target['insertion'] = {'offset': offset, 'length': len(inserted),
                               'hash': hashlib.sha256(inserted.encode()).hexdigest()}
        update(lambda targets: [target if item['registration'] == target['registration'] else item for item in targets])
        # XTEST sync acknowledges the X server, not the application. Retain
        # pending coverage on timeout; captures can bind later full delivery.
        for _ in range(20):
            if bind_delivery(node, target): return
            time.sleep(.025)
    except Exception: pass


def matches(node, target):
    import pyatspi
    try:
        if identity(node) != {key:target[key] for key in ('pid','started','path')}: return False
        if target.get('pending') and not bind_delivery(node, target): return True
        if node.getRole() == pyatspi.ROLE_PASSWORD_TEXT:
            return node.queryText().characterCount == target['length'] and target['length'] > 0
        value = field_value(node)
        if not value: return False
        if target.get('value_hash') is None:
            insertion = target['insertion']
            offset, length = insertion['offset'], insertion['length']
            if len(value) != target['length'] or hashlib.sha256(value[offset:offset+length].encode()).hexdigest() != insertion['hash']: return False
            # First reveal binds the complete contents. Later edits, including
            # same-length prefix/suffix replacement, use that exact fingerprint.
            target['value_hash'] = hashlib.sha256(value.encode()).hexdigest()
            del target['insertion']
            update(lambda targets: [target if item['registration'] == target['registration'] else item for item in targets])
        return hashlib.sha256(value.encode()).hexdigest() == target['value_hash']
    except Exception: raise ValueError('Vault fill field unavailable') from None


def regions():
    """Mask each still-live filled plain entry, retiring removed/replaced entries."""
    targets = read()
    if not targets: return []
    import pyatspi
    try:
        from Xlib import X, display, error
    except ModuleNotFoundError:
        from selkies.Xlib import X, display, error
    connection = open_display(display)
    desktop = pyatspi.Registry.getDesktop(0)
    retained, boxes = [], []
    try:
        for target in targets:
            try:
                window = connection.create_resource_object('window', target['window'])
                owner = window.get_full_property(connection.intern_atom('_NET_WM_PID'), X.AnyPropertyType)
                if owner is None or len(owner.value) != 1: raise ValueError('Vault fill field unavailable')
                if int(owner.value[0]) != target['pid']: continue  # Reused XID retires the old field.
                if window.get_attributes().map_state != X.IsViewable:
                    retained.append(target); continue
                app = next((desktop.getChildAtIndex(i) for i in range(desktop.childCount)
                            if desktop.getChildAtIndex(i).get_process_id() == target['pid']), None)
                if app is None: raise ValueError('Vault fill field unavailable')
                pending = [app]; node = None; visited = 0
                incomplete = False
                while pending and visited < 8192:
                    item = pending.pop(); visited += 1
                    if not item:
                        incomplete = True; continue
                    if item.path == target['path']: node = item; break
                    count = item.childCount
                    limit = min(count,8192-visited)
                    incomplete |= limit < count
                    pending.extend(item.getChildAtIndex(i) for i in range(limit))
                if node is None:
                    if pending or incomplete: raise ValueError('Vault fill field unavailable')
                    continue
                if not matches(node, target): continue
                retained.append(target)
                if node.getRole() == pyatspi.ROLE_PASSWORD_TEXT: continue
                if not node.getState().contains(pyatspi.STATE_SHOWING): continue
                bounds = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                # GTK screen coordinates are DIP at scale2; match its owning
                # frame to the actual X11 client extent before placing pixels.
                frame = node
                while frame and frame.getRoleName() not in ('frame','window','dialog'): frame = frame.parent
                scale = 1
                if frame:
                    logical = frame.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                    geometry = window.get_geometry()
                    if logical.width > 0:
                        ratio = geometry.width/logical.width
                        if abs(ratio-2) < .02: scale = 2
                x,y,w,h = [round(v*scale) for v in (bounds.x,bounds.y,bounds.width,bounds.height)]
                pad = 2*scale
                screen = connection.screen()
                left, top = max(0,x-pad), max(0,y-pad)
                right, bottom = min(screen.width_in_pixels,x+w+pad), min(screen.height_in_pixels,y+h+pad)
                if right > left and bottom > top: boxes.append([left,top,right-left,bottom-top])
            except error.BadWindow: continue  # A destroyed window retires its exact field.
            except Exception: raise ValueError('Vault fill field unavailable') from None
    finally: connection.close()
    if retained != targets:
        removed = {t['registration'] for t in targets if t not in retained}
        update(lambda current: [t for t in current if t['registration'] not in removed])
    return boxes
