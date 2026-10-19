"""MP-08 / MP-11: one native clipboard policy for reads and agent paste."""
# MP-08/MP-11: do not connect an owned display number to a host filesystem socket.
import importlib.util as _x11_import
from pathlib import Path as _X11Path
_x11_spec=_x11_import.spec_from_file_location('native_x11',_X11Path(__file__).with_name('native-x11.py'))
_x11_module=_x11_import.module_from_spec(_x11_spec);_x11_spec.loader.exec_module(_x11_module)

import hashlib
import subprocess
import base64
import json
import os
import tempfile
import stat
from urllib.parse import quote
_fill_spec=_x11_import.spec_from_file_location("native_clipboard_fills",_X11Path(__file__).with_name("native-fill-targets.py"))
_fills=_x11_import.module_from_spec(_fill_spec);_fill_spec.loader.exec_module(_fills)


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


def owner_identity(connection, selection='CLIPBOARD'):
    owner=connection.get_selection_owner(connection.intern_atom(selection))
    return (owner.id,owner_pid(connection,owner)) if owner else None


def clipboard_source(connection, processes, tree, accessibility, selection='CLIPBOARD'):
    # X selections have no provenance by default. Only a live owned native app
    # whose accessibility nodes are all public may supply an agent clipboard.
    identity=owner_identity(connection,selection)
    if identity is None or identity[1] is None:return None
    # MP-11 #904 review 1: retained public nodes of a truncated walk can hide
    # a protected subtree of the owner; only a complete traversal proves it.
    if not tree.get('available') or tree.get('traversed') is not True:return None
    pid=identity[1]
    process=next((p for p in processes if p['pid']==pid),None)
    nodes=[n for n in tree.get('nodes',[]) if n.get('pid')==pid]
    if not process or not accessibility.alive(process) or not nodes or any(n.get('protected') or n.get('browser') for n in nodes):return None
    sensitive=getattr(accessibility,'sensitive_processes',None)
    if sensitive and (pid,str(process['started'])) in sensitive():return None
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


def protected_variants(values):
    for value in values:
        if not isinstance(value,str) or not value:continue
        encoded=value.encode();url=quote(value,safe="~!*'()")
        html=value.replace('&','&amp;').replace('<','&lt;').replace('>','&gt;').replace('"','&quot;').replace("'",'&#39;')
        for variant in (value,value.lower(),value.upper(),url,url.lower(),json.dumps(value,ensure_ascii=False)[1:-1],html,
                        base64.b64encode(encoded).decode(),base64.urlsafe_b64encode(encoded).decode().rstrip('='),encoded.hex(),encoded.hex().upper()):
            yield variant.encode()


def selection_snapshot(values=(),selection='clipboard'):
    stamp=subprocess.run(['xclip','-selection',selection,'-t','TIMESTAMP','-o'],check=True,capture_output=True,timeout=2).stdout
    text=subprocess.run(['xclip','-selection',selection,'-o'],capture_output=True,timeout=2)
    if values and (text.returncode!=0 or any(value in text.stdout for value in protected_variants(values))):
        raise ValueError('protected clipboard contents')
    data=text.stdout if text.returncode==0 else None
    return hashlib.sha256(stamp+b'\0'+(data if data is not None else b'\1')).hexdigest(),data


def selection_fingerprint(values=(),selection='clipboard'):
    return selection_snapshot(values,selection)[0]


def public_selection(connection,processes,tree,accessibility,selection,values):
    # An app may contain a password control and an ordinary username. Prove
    # these exact bytes public; app-wide clipboard admission remains refused.
    identity=owner_identity(connection,selection)
    if identity is None or identity[1] is None or not tree.get('available') or tree.get('traversed') is not True:return None
    process=next((p for p in processes if p['pid']==identity[1]),None)
    if not process or process.get('browser') or not accessibility.alive(process):return None
    fingerprint,data=selection_snapshot(values,selection.lower())
    if data is None:return None
    source=(identity[0],identity[1],process['started'])
    digest=hashlib.sha256(data).hexdigest()
    policy=hashlib.sha256(json.dumps(sorted(set(values)),ensure_ascii=False).encode()).hexdigest()
    path=_fills.store_path();path=path.with_suffix('.public-selections.json') if path is not None else None
    proofs={}
    if path is not None and path.exists():
        metadata=path.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid!=os.getuid() or metadata.st_mode & 0o077:
            raise ValueError('private native clipboard proof required')
        try:
            loaded=json.loads(path.read_text())
            if isinstance(loaded,dict):
                proofs={key:item for key,item in loaded.items() if key in ['CLIPBOARD','PRIMARY'] and isinstance(item,dict) and set(item)=={'source','initial','current','policy'} and isinstance(item['source'],list) and len(item['source'])==3 and all(isinstance(item[k],str) and len(item[k])==64 and all(c in '0123456789abcdef' for c in item[k]) for k in ['initial','current','policy'])}
        except (ValueError,OSError):pass
    previous=proofs.get(selection,{})
    cached=previous.get('source')==list(source) and previous.get('policy')==policy and digest in [previous.get('initial'),previous.get('current')]
    variants=tuple(protected_variants(values))
    # GTK may retain a PRIMARY selection containing only the password dots
    # already visible on screen. These bytes cannot reveal the Vault value;
    # selection_snapshot still rejects any actual known secret first.
    try:
        mask=data.decode('utf-8')
        visible_dots=bool(mask) and len(set(mask))==1 and mask[0] in ('●','•') and any(node.get('pid')==identity[1] and node.get('protected') and node.get('role')=='password text' and 'showing' in node.get('states',[]) for node in tree.get('nodes',[]))
    except UnicodeDecodeError:visible_dots=False
    public=visible_dots or not data or any(data in str(node.get(field,'')).encode() and not any(secret in str(node.get(field,'')).encode() for secret in variants) for node in tree.get('nodes',[]) if node.get('pid')==identity[1] and not node.get('protected') and 'showing' in node.get('states',[]) for field in ['name','text'])
    if not cached and not public:return None
    if not cached and path is not None:
        initial=previous.get('initial') if previous.get('source')==list(source) and previous.get('policy')==policy else digest
        proofs[selection]={'source':list(source),'initial':initial,'current':digest,'policy':policy}
        fd,tmp=tempfile.mkstemp(prefix=path.name+'.',dir=path.parent)
        try:
            with os.fdopen(fd,'w') as output:json.dump(proofs,output)
            os.replace(tmp,path)
        finally:
            if os.path.exists(tmp):os.unlink(tmp)
    return source,fingerprint


def input_admission(processes, accessibility, browser_processes=None, values=()):
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
                selections=[]
                # MP-11: Shift+Insert and native Paste actions may use PRIMARY.
                # A registered value cannot escape through that second selection.
                for selection in ('CLIPBOARD','PRIMARY') if values else ('CLIPBOARD',):
                    identity=owner_identity(connection,selection)
                    if identity is None:
                        selections.append(None);continue
                    tree=accessibility.snapshot(processes,browser_processes)
                    source=clipboard_source(connection,processes,tree,accessibility,selection)
                    if source is None:
                        proved=public_selection(connection,processes,tree,accessibility,selection,values)
                        if proved is None:raise accessibility.NativeInputDenied('native clipboard source protected or unknown')
                        source,fingerprint=proved
                    else:fingerprint=selection_fingerprint(values,selection.lower()) if values else selection_fingerprint()
                    if source[:2]!=identity:raise accessibility.NativeInputDenied('native clipboard source changed')
                    selections.append((identity,fingerprint))
                return tuple(selections) if values else selections[0]
            finally:connection.close()
        except accessibility.NativeInputDenied:raise
        except Exception as error:raise accessibility.NativeInputDenied('native clipboard protection unavailable') from error
    expected=state()
    def check():
        current=state()
        if values:
            # PRIMARY is the UI's live selection and changes during editing.
            # Re-prove every byte/source on every press; freeze CLIPBOARD only.
            changed=current[0]!=expected[0]
        else:changed=current!=expected
        if changed:raise accessibility.NativeInputDenied('native clipboard changed before input')
    return check
