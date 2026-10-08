"""MP-08 / MP-11: one native clipboard policy for reads and agent paste."""
import subprocess


def is_paste_chord(value):
    names={part.lower() for part in value.split('+')}
    control=bool(names & {'ctrl','control','control_l','control_r','super','super_l','super_r','meta','meta_l','meta_r'})
    shift=bool(names & {'shift','shift_l','shift_r'})
    return 'paste' in names or 'xf86paste' in names or ('v' in names and control) or (bool(names & {'insert','kp_insert'}) and shift)


def clipboard_source(connection, processes, tree, accessibility):
    # X selections have no provenance by default. Only a live owned native app
    # with fully public accessibility coverage may supply an agent clipboard.
    owner=connection.get_selection_owner(connection.intern_atom('CLIPBOARD'))
    if not owner:return None
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
        pid=pids[0]
        prop=owner.get_full_property(connection.intern_atom('_NET_WM_PID'),0)
        if prop is not None and (prop.format!=32 or len(prop.value)!=1 or int(prop.value[0])!=pid):return None
    except Exception:return None
    process=next((p for p in processes if p['pid']==pid),None)
    nodes=[n for n in tree.get('nodes',[]) if n.get('pid')==pid]
    if not process or not accessibility.alive(process) or not nodes or any(n.get('protected') for n in nodes):return None
    return (owner.id,pid,process['started'])


def public_clipboard(processes, accessibility, mask=False, browser_processes=None):
    before=accessibility.snapshot(processes,browser_processes)
    if mask or not before['available'] or not before['complete'] or before['protected'] or before.get('uncovered') or any(n.get('protected') for n in before.get('nodes',[])):return None
    if not processes or not before.get('nodes'):return None
    try:
        from Xlib import display
    except ModuleNotFoundError:
        from selkies.Xlib import display
    connection=None
    try:
        connection=display.Display()
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


def paste_guard(processes, accessibility, browser_processes=None):
    def read():
        try:value=public_clipboard(processes,accessibility,browser_processes=browser_processes)
        except Exception as error:raise accessibility.NativeInputDenied('native clipboard protection unavailable') from error
        if value is None:raise accessibility.NativeInputDenied('native clipboard source protected or unknown')
        return value
    expected=read()
    def check():
        if read()!=expected:raise accessibility.NativeInputDenied('native clipboard changed before paste')
    return check
