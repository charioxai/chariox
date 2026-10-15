"""MP-08/MP-10/MP-11: recipient admission for physical agent input."""
import importlib.util
from pathlib import Path
_spec=importlib.util.spec_from_file_location("native_clipboard",Path(__file__).with_name("native-clipboard.py"))
_clipboard=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(_clipboard)
owner_pid=_clipboard.owner_pid
from urllib.parse import urlsplit


def browser_document_point(connection, window, pid, point, accessibility):
    """Prove a page hit inside this exact X client, never Chrome's controls."""
    api=accessibility.pyatspi
    root=connection.screen().root
    geometry=window.get_geometry();origin=root.translate_coords(window,0,0)
    client=[origin.x,origin.y,geometry.width,geometry.height]
    frame_rect=accessibility.frame_rect(root,window)
    desktop=api.Registry.getDesktop(0);frames=[]
    if desktop.childCount>64:return False
    for i in range(desktop.childCount):
        app=desktop.getChildAtIndex(i)
        if not app or app.get_process_id()!=pid:continue
        if app.childCount>64:return False
        for j in range(app.childCount):
            frame=app.getChildAtIndex(j)
            if not frame or frame.getRoleName() not in ('frame','window') or not frame.getState().contains(api.STATE_SHOWING):continue
            rect=frame.queryComponent().getExtents(api.DESKTOP_COORDS)
            scale=accessibility.native_frame_scale([rect.x,rect.y,rect.width,rect.height],frame_rect,client)
            if scale:frames.append((frame,scale))
    if len(frames)!=1:return False
    frame,scale=frames[0];x,y=(int(value/scale) for value in point);node=frame
    for _ in range(64):
        try:child=node.queryComponent().getAccessibleAtPoint(x,y,api.DESKTOP_COORDS)
        except NotImplementedError:break
        if child is None or child==node:break
        node=child
    else:return False
    document=False
    for _ in range(64):
        if node==frame:return document
        if not node:return False
        if node.getRoleName()=='document web':
            # In-window browser WebUI (omnibox, install dialogs) is not a web
            # page. Check every nested document, including extension frames.
            uri=node.queryDocument().getAttributeValue('URI')
            if urlsplit(uri).scheme not in ('http','https'):return False
            document=True
        node=node.parent
    return False


class NativeExtensionDenied(ValueError):
    """MP-11: native Chrome controls have no document-bound agent authority."""


def input_destination(connection, processes, accessibility, point=None):
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
                if point is not None and browser_document_point(connection,window,pid,point,accessibility):return
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
