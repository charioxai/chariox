"""MP-08/MP-10/MP-11: recipient admission for physical agent input."""
import importlib.util
from pathlib import Path
_spec=importlib.util.spec_from_file_location("native_clipboard",Path(__file__).with_name("native-clipboard.py"))
_clipboard=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(_clipboard)
owner_pid=_clipboard.owner_pid
class NativeExtensionDenied(ValueError):
    """MP-11: global extension accelerators require owner authority."""


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
