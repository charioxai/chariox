"""MP-08/MP-10/MP-11: recipient admission for physical agent input."""
import importlib.util
from pathlib import Path
from urllib.parse import urlsplit
_spec=importlib.util.spec_from_file_location("native_clipboard",Path(__file__).with_name("native-clipboard.py"))
_clipboard=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(_clipboard)
owner_pid=_clipboard.owner_pid
class NativeExtensionDenied(ValueError):
    """MP-11: global extension accelerators require owner authority."""


# Chromium's native widget classes are browser-owned AX metadata, unlike page
# labels. Every document boundary is checked here: the Room action path
# does not pass through the host browser inventory guard.
_NAVIGATION = {'OmniboxViewViews', 'LocationBarView', 'LocationIconView',
               'BackButton', 'ForwardButton', 'ReloadButton', 'ToolbarButton',
               'Tab', 'TabCloseButton', 'TabStripControlButton',
               'TabStripFlatEdgeButton', 'FrameCaptionButton', 'PageActionView',
               'BrowserAppMenuButton', 'OmniboxResultView', 'OmniboxPopupView', 'WebUIToolbarWebView'}
_EXTENSION = {'ToolbarActionView', 'ExtensionsToolbarButton',
              'ExtensionsMenuView', 'ExtensionsMenuItemView'}


def public_document(node):
    """MP-11: use browser-owned document URI, including every nested ancestor."""
    try:
        uri = node.queryDocument().getAttributeValue('URI')
        if not isinstance(uri, str) or not uri or any(ord(c) < 32 for c in uri):
            raise ValueError('document URI unavailable')
        url = urlsplit(uri)
        scheme, host = url.scheme.lower(), (url.hostname or '').lower()
        if not scheme:
            raise ValueError('document URI unavailable')
        extension = scheme == 'chrome-extension' or scheme == 'chrome' and host == 'extensions'
        store = scheme in ('http', 'https') and (host == 'chromewebstore.google.com' or
                host == 'chrome.google.com' and (url.path == '/webstore' or url.path.startswith('/webstore/')))
        if extension or store:
            raise ValueError('extension document')
    except Exception:
        raise NativeExtensionDenied('browser document requires owner input') from None


def browser_control(node, pid):
    """MP-11: admit the actual page/navigation control, never an action icon.

    Walk the whole native ancestry even when a safe-looking descendant exists.
    Labels and worker liveness cannot prove that an extension action is safe.
    """
    chain = []
    for _ in range(48):
        if node is None or node.get_process_id() != pid:
            raise NativeExtensionDenied('browser control unavailable')
        if node.getRoleName() == 'application': break
        chain.append(node)
        node = node.parent
    else: raise NativeExtensionDenied('browser control ancestry unavailable')
    # CSS classes below a document boundary belong to the page. Read only
    # browser-owned widget metadata beyond it; labels never grant authority.
    documents = [i for i, item in enumerate(chain) if item.getRoleName() == 'document web']
    for index in documents:
        public_document(chain[index])
    native = chain[max(documents)+1:] if documents else chain
    public = bool(documents)
    for item in native:
        attrs = dict(part.split(':', 1) for part in item.getAttributes() if ':' in part)
        widget = attrs.get('class', '')
        if widget in _EXTENSION or widget.startswith('Extension'):
            raise NativeExtensionDenied('extension control requires owner input')
        public = public or widget in _NAVIGATION
    if not public: raise NativeExtensionDenied('browser control unavailable')


def browser_frame(accessibility, connection, window, process):
    """MP-08/MP-11: the one AT-SPI frame that is this exact X11 window.

    AT-SPI reports logical extents on a scaled desktop, X11 physical pixels:
    the frame must match the window's geometry under an integer scale.
    """
    atspi = accessibility.pyatspi
    root = connection.screen().root
    geometry = window.get_geometry(); origin = root.translate_coords(window, 0, 0)
    client = [origin.x, origin.y, geometry.width, geometry.height]
    outer = accessibility.frame_rect(root, window)
    desktop = atspi.Registry.getDesktop(0)
    apps = [desktop.getChildAtIndex(i) for i in range(min(desktop.childCount, 64))]
    apps = [app for app in apps if app and app.get_process_id() == process['pid']]
    if len(apps) != 1: raise NativeExtensionDenied('browser app unavailable')
    frames = []
    for i in range(min(apps[0].childCount, 256)):
        frame = apps[0].getChildAtIndex(i)
        if not frame or not frame.getState().contains(atspi.STATE_SHOWING): continue
        box = frame.queryComponent().getExtents(atspi.DESKTOP_COORDS)
        scale = accessibility.native_frame_scale([box.x, box.y, box.width, box.height], outer, client)
        if scale: frames.append((frame, scale))
    if len(frames) != 1: raise NativeExtensionDenied('browser window unavailable')
    return frames[0]


def browser_destination(accessibility, process, point, connection, window):
    atspi = accessibility.pyatspi
    frame, scale = browser_frame(accessibility, connection, window, process)
    if point is not None:
        # The X11 point is physical; hit-test in AT-SPI's logical coordinates.
        x, y = (int(value / scale) for value in point)
        node = frame.queryComponent().getAccessibleAtPoint(x, y, atspi.DESKTOP_COORDS)
        for _ in range(48):
            if node is None: raise NativeExtensionDenied('browser point unavailable')
            child = node.queryComponent().getAccessibleAtPoint(x, y, atspi.DESKTOP_COORDS)
            if child is None or child == node: break
            node = child
        else: raise NativeExtensionDenied('browser point depth exceeded')
    else:
        # The recipient is the focused sink inside the focused X11 window;
        # other windows and unfocused documents are not in its ancestry.
        # No page values or full observation snapshot are read to admit keys.
        pending = [frame]; focused = []; seen = 0
        while pending and seen < 12000:
            candidate = pending.pop(); seen += 1
            state = candidate.getState()
            if state.contains(atspi.STATE_FOCUSED) and state.contains(atspi.STATE_SHOWING):
                focused.append(candidate)
            if candidate.getRoleName() == 'document web': continue
            if candidate.childCount > 12000: raise NativeExtensionDenied('browser focus depth exceeded')
            for i in range(candidate.childCount):
                child = candidate.getChildAtIndex(i)
                if child: pending.append(child)
        if pending or not focused: raise NativeExtensionDenied('browser focus unavailable')
        # Chromium may mark both an iframe and its document focused. Every
        # reported sink must be public; never select one and ignore another.
        for candidate in focused: browser_control(candidate, process['pid'])
        return
    browser_control(node, process['pid'])


def input_destination(connection, processes, accessibility, point=None):
    """MP-08/MP-10/MP-11: prove the actual X input recipient before dispatch.

    Owned Chromium controls and page fields share Computer authority.
    Extension inventory and global accelerators retain their separate fences.
    """
    try:
        if point is None:
            window=connection.get_input_focus().focus
        else:
            pointer=connection.screen().root.query_pointer()
            if [pointer.root_x,pointer.root_y]!=list(point):raise ValueError('pointer moved')
            window=pointer.child
            # Window managers reparent clients; the outer frame is not the
            # destination. XQueryPointer supplies the topmost child at each level.
            for _ in range(32):
                if not window:raise ValueError('pointer destination unavailable')
                child=window.query_pointer().child
                if not child:break
                window=child
            else:raise ValueError('pointer window depth exceeded')
        pid=owner_pid(connection,window)
        process=next((item for item in processes if item['pid']==pid),None)
        if not process or not accessibility.alive(process):raise ValueError('input destination is not owned')
        if process.get('browser'):
            browser_destination(accessibility, process, point, connection, window)
        # MP-08/MP-11: Computer includes the owned browser's address bar and
        # page fields. The controller retains extension admission, and root
        # accelerator probes remain below; field values are observation policy.
    except NativeExtensionDenied:raise
    except Exception as error:
        raise accessibility.NativeInputDenied('native input destination unavailable') from error


def key_destinations(connection, codes):
    """MP-11: reject root accelerators, which bypass the focused X client.

    The caller holds the server grab through this probe and physical dispatch.
    Probe the exact key/modifier sequence, including lock bits, without sending
    an event. BadAccess proves another client owns that passive root grab. A
    successful temporary grab is removed before input; no global grab is stolen.
    """
    from Xlib import X
    root=connection.screen().root
    modifiers=connection.get_modifier_mapping()
    mask=root.query_pointer().mask & 255
    for code in codes:
        errors=[]
        try:
            root.grab_key(code,mask,False,X.GrabModeAsync,X.GrabModeAsync,
                          onerror=lambda error,request:errors.append(error))
            connection.sync()
            if errors:raise NativeExtensionDenied('Global shortcuts require owner input')
        finally:
            root.ungrab_key(code,mask)
            connection.sync()
        for index,row in enumerate(modifiers):
            if code in row:mask |= 1 << index
