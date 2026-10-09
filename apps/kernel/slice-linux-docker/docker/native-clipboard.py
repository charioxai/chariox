"""MP-08 / MP-11: one native clipboard policy for reads and agent paste."""
# MP-08/MP-11: do not connect an owned display number to a host filesystem socket.
import importlib.util as _x11_import
from pathlib import Path as _X11Path
_x11_spec=_x11_import.spec_from_file_location('native_x11',_X11Path(__file__).with_name('native-x11.py'))
_x11_module=_x11_import.module_from_spec(_x11_spec);_x11_spec.loader.exec_module(_x11_module)

import hashlib
import subprocess


def owner_pid(connection, owner):
    # MP-11 review R3: GTK selection windows need not publish _NET_WM_PID.
    # XRes must identify the owning connection; a client claim is only an
    # optional consistency check and can never grant clipboard provenance.
    try:
        if not connection.has_extension('X-Resource'):return None
        version=connection.res_query_version()
        if (version.server_major,version.server_minor)<(1,2):return None
        identities=connection.res_query_client_ids([{'client':owner.id,'mask':2}]).ids
        pids=[int(item.value[0]) for item in identities if item.spec.mask & 2 and len(item.value)==1]
        if len(pids)!=1 or pids[0]<=1:return None
        prop=owner.get_full_property(connection.intern_atom('_NET_WM_PID'),0)
        if prop is not None and (prop.format!=32 or len(prop.value)!=1 or int(prop.value[0])!=pids[0]):return None
        return pids[0]
    except Exception:return None


def owner_identity(connection):
    owner=connection.get_selection_owner(connection.intern_atom('CLIPBOARD'))
    return (owner.id,owner_pid(connection,owner)) if owner else None


def clipboard_source(connection, processes, tree, accessibility):
    # X selections have no provenance by default. Only a live owned native app
    # whose accessibility nodes are all public may supply an agent clipboard.
    identity=owner_identity(connection)
    if identity is None or identity[1] is None:return None
    # MP-11 #904 review 1: retained public nodes of a truncated walk can hide
    # a protected subtree of the owner; only a complete traversal proves it.
    if not tree.get('available') or tree.get('traversed') is not True:return None
    pid=identity[1]
    process=next((p for p in processes if p['pid']==pid),None)
    nodes=[n for n in tree.get('nodes',[]) if n.get('pid')==pid]
    if not process or not accessibility.alive(process) or not nodes or any(n.get('protected') for n in nodes):return None
    return (identity[0],pid,process['started'])


def display_module():
    try:
        from Xlib import display
    except ModuleNotFoundError:
        from selkies.Xlib import display
    return display


def public_clipboard(processes, accessibility, mask=False, browser_processes=None):
    before=accessibility.snapshot(processes,browser_processes)
    if mask or not before['available'] or not before['complete'] or before['protected'] or before.get('uncovered') or any(n.get('protected') for n in before.get('nodes',[])):return None
    if not processes or not before.get('nodes'):return None
    connection=None
    try:
        connection=_x11_module.open_display(display_module())
        source=clipboard_source(connection,processes,before,accessibility)
        if source is None:return None
        result=subprocess.run(['xclip','-selection','clipboard','-o'],check=True,capture_output=True,timeout=2)
        after=accessibility.snapshot(processes,browser_processes)
        if before!=after or source!=clipboard_source(connection,processes,after,accessibility):
            raise ValueError('native protection changed during clipboard read')
        if len(result.stdout)>65536:raise ValueError('clipboard too large')
        return (result.stdout.decode('utf-8'),source)
    finally:
        if connection is not None:connection.close()


def selection_fingerprint():
    """MP-11: fence clipboard replacement, also by the same owner window.

    The ICCCM TIMESTAMP changes whenever an owner re-acquires the selection;
    the text digest covers an owner that serves new bytes without doing so.
    Bytes stay inside this helper; only a digest is compared.
    """
    stamp=subprocess.run(['xclip','-selection','clipboard','-t','TIMESTAMP','-o'],check=True,capture_output=True,timeout=2).stdout
    text=subprocess.run(['xclip','-selection','clipboard','-o'],capture_output=True,timeout=2)
    return hashlib.sha256(stamp+b'\0'+(text.stdout if text.returncode==0 else b'\1')).hexdigest()


def input_admission(processes, accessibility, browser_processes=None):
    """MP-11: any agent key, text, click or action may reach a Paste control.

    Admit an empty CLIPBOARD or one owned by a proved, completely traversed,
    public owned app. Other windows' coverage does not change what a paste
    inserts, so it is not checked. The returned fence runs before every press:
    an empty selection stays empty (cheap); otherwise the source is proved
    again and its selection must not have been replaced.
    """
    def state():
        try:
            connection=_x11_module.open_display(display_module())
            try:
                identity=owner_identity(connection)
                if identity is None:return None
                source=clipboard_source(connection,processes,accessibility.snapshot(processes,browser_processes),accessibility)
                if source is None or source[:2]!=identity:raise accessibility.NativeInputDenied('native clipboard source protected or unknown')
                return (identity,selection_fingerprint())
            finally:connection.close()
        except accessibility.NativeInputDenied:raise
        except Exception as error:raise accessibility.NativeInputDenied('native clipboard protection unavailable') from error
    expected=state()
    def check():
        if state()!=expected:raise accessibility.NativeInputDenied('native clipboard changed before input')
    return check
