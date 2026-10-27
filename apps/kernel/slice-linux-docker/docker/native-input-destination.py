"""MP-08/MP-10/MP-11: recipient admission for physical agent input."""
import importlib.util
from pathlib import Path
_spec=importlib.util.spec_from_file_location("native_clipboard",Path(__file__).with_name("native-clipboard.py"))
_clipboard=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(_clipboard)
owner_pid=_clipboard.owner_pid
from urllib.parse import urlsplit


def browser_document_point(connection, window, point, page, accessibility):
    """Prove a page hit inside this exact X client, never Chrome's controls.

    page: the kernel's CDP view of the active Tab (URL, window bounds, native
    viewport size). Its rectangle must be proven by the live X11 geometry of
    the window under the pointer; anything else is browser UI. A visible in-window
    list (omnibox) shares the X client and the Tab rectangle: no overlay proof, no page hit.
    """
    if not isinstance(page,dict) or not isinstance(page.get('url'),str) or urlsplit(page['url']).scheme not in ('http','https') or page.get('overlay') is not False:return False
    root=connection.screen().root
    geometry=window.get_geometry();origin=root.translate_coords(window,0,0)
    client=[origin.x,origin.y,geometry.width,geometry.height]
    rect=accessibility._protection.viewport_rect(page,client,accessibility.frame_insets(connection,window))
    return rect is not None and rect[0]<=point[0]<rect[0]+rect[2] and rect[1]<=point[1]<rect[1]+rect[3]


class NativeExtensionDenied(ValueError):
    """MP-11: native Chrome controls have no document-bound agent authority."""


def input_destination(connection, processes, accessibility, point=None, page=None):
    """MP-08/MP-10/MP-11: prove the actual X input recipient before dispatch.

    A toolbar action can run an extension without opening a popup or worker.
    Chrome controls require Browser authority; a proved ordinary page hit keeps
    the native Computer fallback. Human input keeps its own path.
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
            try:
                if point is not None and browser_document_point(connection,window,point,page,accessibility):return
            except Exception:pass  # Unresolved native controls cannot authorize an extension.
            raise NativeExtensionDenied('Browser controls require document-bound Browser input')
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
