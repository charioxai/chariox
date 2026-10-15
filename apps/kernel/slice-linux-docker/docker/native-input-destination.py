"""MP-08/MP-10/MP-11: recipient admission for physical agent input."""
import importlib.util
from pathlib import Path
_spec=importlib.util.spec_from_file_location("native_clipboard",Path(__file__).with_name("native-clipboard.py"))
_clipboard=importlib.util.module_from_spec(_spec);_spec.loader.exec_module(_clipboard)
owner_pid=_clipboard.owner_pid


class NativeExtensionDenied(ValueError):
    """MP-11: native Chrome controls have no document-bound agent authority."""


def input_destination(connection, processes, accessibility, point=None):
    """MP-08/MP-10/MP-11: prove the actual X input recipient before dispatch.

    A toolbar action can run an extension without opening a popup or worker.
    Native Computer input cannot distinguish/authorize these Chrome controls;
    agents use document-bound Browser input. Human input keeps its own path.
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
            raise NativeExtensionDenied('Browser controls require document-bound Browser input')
    except NativeExtensionDenied:raise
    except Exception as error:
        raise accessibility.NativeInputDenied('native input destination unavailable') from error

