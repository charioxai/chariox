"""MP-08 / MP-11: private-session AT-SPI tree; only verified owned applications."""
# MP-08/MP-11: do not connect an owned display number to a host filesystem socket.
import importlib.util as _x11_import
from pathlib import Path as _X11Path
_x11_spec=_x11_import.spec_from_file_location('native_x11',_X11Path(__file__).with_name('native-x11.py'))
_x11_module=_x11_import.module_from_spec(_x11_spec);_x11_spec.loader.exec_module(_x11_module)
# MP-08/MP-11: proven CDP document-to-desktop placement for the kernel browser.
_protection_spec=_x11_import.spec_from_file_location('browser_desktop_protection',_X11Path(__file__).with_name('browser-desktop-protection.py'))
_protection=_x11_import.module_from_spec(_protection_spec);_protection_spec.loader.exec_module(_protection)

import hashlib
import json
import os
import pyatspi
from collections import deque
from pathlib import Path
from types import SimpleNamespace
# MP-08 / MP-10 / MP-11: private protection coverage is independent of
# the 64-node / 3 KiB public projection. Exhaustion still masks captures.
MAX_NODES=8192
MAX_DEPTH=32


class NativeInputDenied(ValueError):
    """MP-11: trusted helper refusal maps to the existing Browser/Vault code."""
    pass


def alive(process):
    try:
        stat=open('/proc/'+str(process['pid'])+'/stat').read()
        return (stat[stat.rfind(')')+2:].split()[19]==process['started'] and
                os.stat('/proc/'+str(process['pid'])).st_uid==os.getuid())
    except (OSError,KeyError,IndexError):return False



def visible_table_children(node, bounds):
    """MP-08 / MP-10 / MP-11: cover a managed virtual table's visible pixels.

    AT-SPI MANAGES_DESCENDANTS tables may advertise billions of virtual cells.
    Walk their actual screen rectangles, rather than enumerating invisible cells.
    Every covered rectangle must resolve to a showing, direct table cell; gaps,
    invalid geometry and budget exhaustion leave capture protection uncertain.
    """
    if not bounds or any(not isinstance(value, int) for value in bounds):
        raise ValueError('table geometry unavailable')
    x, y, width, height = bounds
    if x < 0 or y < 0 or width <= 0 or height <= 0 or x+width > 16384 or y+height > 16384:
        raise ValueError('table geometry out of bounds')
    component = node.queryComponent()
    result = []; seen = set(); bottom = y+height; right = x+width
    while y < bottom:
        column = x; next_row = bottom
        while column < right:
            if len(result) >= MAX_NODES:
                raise ValueError('visible table budget exhausted')
            child = component.getAccessibleAtPoint(column, y, pyatspi.DESKTOP_COORDS)
            if child is None or child.getRole() != pyatspi.ROLE_TABLE_CELL or not child.getState().contains(pyatspi.STATE_SHOWING):
                raise ValueError('visible table coverage unavailable')
            index = child.getIndexInParent()
            if index < 0 or index >= node.childCount:
                raise ValueError('visible table child changed')
            rect = child.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            indexed = node.getChildAtIndex(index)
            # Managed descendants may be recreated with different object paths.
            # Bind the indexed target to the same visible cell geometry/content.
            if indexed is None or indexed.getRole() != pyatspi.ROLE_TABLE_CELL or not indexed.getState().contains(pyatspi.STATE_SHOWING) or indexed.name != child.name:
                raise ValueError('visible table child changed')
            indexed_rect = indexed.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            if (indexed_rect.x, indexed_rect.y, indexed_rect.width, indexed_rect.height) != (rect.x, rect.y, rect.width, rect.height):
                raise ValueError('visible table child geometry changed')
            child = indexed
            if rect.width <= 0 or rect.height <= 0 or not (rect.x <= column < rect.x+rect.width and rect.y <= y < rect.y+rect.height):
                raise ValueError('visible table geometry changed')
            if index not in seen:
                result.append((index, child)); seen.add(index)
            column = min(right, rect.x+rect.width)
            next_row = min(next_row, rect.y+rect.height)
        y = next_row
    return result


def frame_rect(root, window):
    """MP-08: the whole window-manager frame (title included) of a client window."""
    for _ in range(16):
        parent=window.query_tree().parent
        if parent.id==root.id:
            geometry=window.get_geometry()
            return [geometry.x,geometry.y,geometry.width+2*geometry.border_width,geometry.height+2*geometry.border_width]
        window=parent
    raise ValueError('window frame unavailable')


def visible_rect(rect, screen):
    # MP-11 review R3: GTK owns an offscreen selection window. Only actual
    # desktop pixels need coverage; partially visible popups remain masked.
    x,y,width,height=rect
    left,top=max(0,x),max(0,y)
    right,bottom=min(screen.width_in_pixels,x+width),min(screen.height_in_pixels,y+height)
    return [left,top,right-left,bottom-top] if right>left and bottom>top else None


def subtract(rect, cover):
    """MP-08: parts of rect [x,y,w,h] not hidden by an opaque cover rect."""
    x,y,w,h=rect;cx,cy,cw,ch=cover
    if cx>=x+w or cx+cw<=x or cy>=y+h or cy+ch<=y:return [rect]
    top,bottom=max(y,cy),min(y+h,cy+ch)
    parts=[[x,y,w,cy-y]] if cy>y else []
    if cy+ch<y+h:parts.append([x,cy+ch,w,y+h-cy-ch])
    if cx>x:parts.append([x,top,cx-x,bottom-top])
    if cx+cw<x+w:parts.append([cx+cw,top,x+w-cx-cw,bottom-top])
    return parts


def value_boxes(node, values, pyatspi):
    """MP-08 / MP-11 (owner 2026-10-09): Vault never blacks out the desktop. A
    registered value shown as accessible text is masked by its text range (or
    the node box for a name); anything that cannot be checked is left alone."""
    boxes=[]
    if not values:return boxes
    try:
        text=node.queryText();content=text.getText(0,min(text.characterCount,65536))
        for value in values:
            start=content.find(value)
            while start>=0 and len(boxes)<16:
                try:
                    x,y,width,height=text.getRangeExtents(start,start+len(value),pyatspi.DESKTOP_COORDS)
                    if width>0 and height>0:boxes.append([x,y,width,height])
                except Exception:pass
                start=content.find(value,start+len(value))
    except Exception:pass
    if any(value in (node.name or '') for value in values):
        try:
            rect=node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            if rect.width>0 and rect.height>0:boxes.append([rect.x,rect.y,rect.width,rect.height])
        except Exception:pass
    return boxes


def snapshot(processes, browser_processes=None, browser_protection=None, values=()):
    allowed={item['pid']:item['started'] for item in processes if alive(item)}
    browsers={item['pid'] for item in browser_processes or () if alive(item)}
    # Only the kernel's own Chromium tree was measured through CDP.
    measured=set(browsers) if browser_protection is not None else set()
    documents={};withheld=0
    # MP-08 / MP-11: slice placement does not own Chromium's launch object.
    # Kernel host placement supplies the full tracked browser process tree.
    # OS executable metadata and document-web roles conservatively protect
    # additional browser apps; they never grant coverage or input authority.
    for pid in allowed:
        try:
            binary=os.path.basename(os.readlink('/proc/'+str(pid)+'/exe')).lower()
            if 'chrome' in binary or 'chromium' in binary or 'firefox' in binary:browsers.add(pid)
        except OSError:pass
    # MP-11: `complete` also requires every window attributed (capture masking);
    # `traversed` only that the owned AT-SPI trees were walked without truncation.
    nodes=[];complete=traversed=True;protected=False;pending=deque();uncovered=[];masks=[];echoes=[]
    try: desktop=pyatspi.Registry.getDesktop(0)
    except Exception:return {'available':False,'complete':False,'nodes':[],'protected':True}
    def visit(node,pid,started,path,depth):
        nonlocal complete,traversed,protected
        if depth>MAX_DEPTH or len(nodes)>=MAX_NODES:complete=traversed=False;return
        role=node.getRoleName()
        secret=node.getRole()==pyatspi.ROLE_PASSWORD_TEXT
        protected=protected or secret
        state=node.getState()
        states=[label for flag,label in [(pyatspi.STATE_SHOWING,'showing'),(pyatspi.STATE_ENABLED,'enabled'),(pyatspi.STATE_FOCUSED,'focused'),(pyatspi.STATE_EDITABLE,'editable')] if state.contains(flag)]
        bounds=None
        try:
            rect=node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            bounds=[rect.x,rect.y,rect.width,rect.height]
        except NotImplementedError:pass
        actions=[]
        if not secret:
            try:
                action=node.queryAction()
                actions=[action.getName(i) for i in range(min(action.nActions,16))]
            except NotImplementedError:pass
        echoes.extend([] if secret else value_boxes(node,values,pyatspi))
        name='[protected]' if secret else (node.name or '')[:4096]
        for value in values:name=name.replace(value,'[redacted]')
        nodes.append({'pid':pid,'started':started,'path':path,'role':role,'name':name,'states':states,'bounds':bounds,'actions':actions,'protected':secret})
        if not secret:
            managed_table = (node.getRole() == pyatspi.ROLE_TABLE and
                             state.contains(pyatspi.STATE_MANAGES_DESCENDANTS) and
                             node.childCount > MAX_NODES)
            # MP-08 / MP-11: finite GTK tables include headers, empty space and
            # hidden views without screen geometry. Cover all their children,
            # including password descendants; only huge virtual tables need
            # the verified visible-cell walk.
            try:
                children = visible_table_children(node, bounds) if managed_table else (
                    (i, node.getChildAtIndex(i)) for i in range(min(node.childCount, MAX_NODES)))
                for i, child in children:
                    if len(nodes)+len(pending)>=MAX_NODES:
                        complete=traversed=False
                        break
                    if child:pending.append((child,pid,started,path+[i],depth+1))
                if not managed_table and node.childCount>MAX_NODES:complete=traversed=False
            except (ValueError, NotImplementedError):
                complete=traversed=False
    try:
        for app_index in range(min(desktop.childCount,64)):
            app=desktop.getChildAtIndex(app_index)
            if not app:continue
            pid=app.get_process_id()
            if pid not in allowed:
                complete=False
                continue
            if pid in browsers:
                # MP-08 / MP-11: native observation already masks this entire
                # browser. Do not inspect its private, potentially huge tree.
                nodes.append({'pid':pid,'started':allowed[pid],'path':[],
                              'role':'application','name':'[protected]','states':[],
                              'bounds':None,'actions':[],'protected':True})
                if pid in measured:
                    try:documents[pid]=_protection.document_rects(app,pyatspi)
                    except Exception:pass  # Unproven document geometry: whole window.
                continue
            pending.append((app,pid,allowed[pid],[],0))
        if desktop.childCount>64:complete=traversed=False
        # MP-08: traverse applications fairly before deep hidden menu trees.
        while pending and len(nodes)<MAX_NODES:
            visit(*pending.popleft())
        if pending:complete=traversed=False
        browsers.update(node['pid'] for node in nodes if node['role']=='document web')
        # A private bus alone does not prove all visible windows expose AT-SPI.
        # Unknown/unscoped native windows make password coverage uncertain.
        try:
            from Xlib import X, display
        except ModuleNotFoundError as error:
            if error.name != 'Xlib': raise
            from selkies.Xlib import X, display
        connection=_x11_module.open_display(display)
        active_window=None
        try:
            screen=connection.screen();root=screen.root
            active=root.get_full_property(connection.intern_atom('_NET_ACTIVE_WINDOW'),X.AnyPropertyType)
            active_id=int(active.value[0]) if active is not None and len(active.value) else None
            # MP-08: bottom-to-top order lets capture skip parts hidden by owned windows above.
            clients=root.get_full_property(connection.intern_atom('_NET_CLIENT_LIST_STACKING'),X.AnyPropertyType)
            stacked=clients is not None
            if not stacked:clients=root.get_full_property(connection.intern_atom('_NET_CLIENT_LIST'),X.AnyPropertyType)
            windows=[]
            for window_id in clients.value if clients is not None else []:
                window=connection.create_resource_object('window',int(window_id))
                if window.get_attributes().map_state!=X.IsViewable:continue
                pid=window.get_full_property(connection.intern_atom('_NET_WM_PID'),X.AnyPropertyType)
                pid=int(pid.value[0]) if pid is not None and len(pid.value) else None
                rect=frame_rect(root,window)
                geometry=window.get_geometry()
                origin=root.translate_coords(window,0,0)
                client_rect=[origin.x,origin.y,geometry.width,geometry.height]
                title=window.get_full_property(connection.intern_atom('_NET_WM_NAME'),X.AnyPropertyType)
                name=bytes(title.value).decode('utf-8',errors='replace') if title is not None else window.get_wm_name()
                # MP-11: PID membership alone is never window coverage. Match
                # a showing frame's title AND actual X11 screen geometry.
                frames=[node for node in nodes if node['pid']==pid and node['role'] in ('frame','window','dialog') and
                        node['name']==name and 'showing' in node['states'] and node['bounds'] in (rect,client_rect)]
                windows.append((int(window_id),pid,rect,frames,client_rect))
            for window_id,pid,rect,frames,client in windows:
                # MP-11 #904 review 3: the frame proof needs the owned trees fully
                # traversed, not unrelated windows' capture coverage (masks still
                # require 'complete' in every pixel consumer).
                frame=frames[0] if traversed and len(frames)==1 else None
                # A single accessible frame cannot authorize two X windows.
                covered=frame is not None and sum(any(node is frame for node in candidates) for _,_,_,candidates,_ in windows)==1
                if pid not in allowed:complete=False
                if not covered or pid in browsers:
                    visible=visible_rect(rect,screen)
                    # MP-08/MP-11: a proven kernel-browser window masks only its
                    # protected regions; any unbound window stays withheld whole.
                    precise=_protection.window_masks(browser_protection,client,rect,documents[pid]) if pid in documents else None
                    withheld+=pid in measured and precise is None
                    if visible:
                        uncovered.append(visible)
                        masks.extend([visible] if precise is None else [part for part in (visible_rect(mask,screen) for mask in precise) if part])
                elif stacked and masks:
                    masks=[part for region in masks for part in subtract(region,rect)]
                if covered and window_id==active_id:
                    active_window={key:frame[key] for key in ('pid','started','path')}
            # MP-08 / MP-11: menus, completion lists and tooltips are
            # override-redirect root children outside _NET_CLIENT_LIST with no
            # reliable owner. Mask every viewable one (over-masking is accepted).
            for child in root.query_tree().children:
                attributes=child.get_attributes()
                if attributes.override_redirect and attributes.map_state==X.IsViewable:
                    geometry=child.get_geometry()
                    visible=visible_rect([geometry.x,geometry.y,geometry.width+2*geometry.border_width,geometry.height+2*geometry.border_width],screen)
                    if visible:uncovered.append(visible);masks.append(visible)
        finally:connection.close()
        # MP-11: browser pixels are placed by the CDP transform above; the
        # structured browser app stays withheld (titles, OTP/payment/private
        # ancestors, nested frames and shadow content are never AT-SPI data).
        # The before/after capture fence includes these masks and identities.
        for node in nodes:
            if node['pid'] in browsers:
                node['native_protected']=node['protected']
                node.update(name='[protected]',actions=[],protected=True)
        # Best-effort Vault echo boxes, clipped to the screen (owner 2026-10-09).
        masks.extend(part for part in (visible_rect(box,screen) for box in echoes) if part)
        return {'available':True,'complete':complete,'traversed':traversed,'nodes':nodes,'protected':protected,'active_window':active_window,'uncovered':uncovered,'masks':masks,'browser_withheld':withheld}
    except Exception:
        # Partial traversal cannot establish native password/pixel coverage.
        return {'available':False,'complete':False,'nodes':[],'protected':True}


def tree_digest(tree):
    return hashlib.sha256(json.dumps(tree,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()).hexdigest()


def input_target(processes, expected=None):
    """MP-08 / MP-11: ordinary text requires a proved, public native focus.

    AT-SPI cannot prove a Browser DOM leaf's Vault/private attributes. Browser
    web input uses the existing document-bound Browser path instead; this
    native fallback refuses it, including nested frame/shadow descendants.
    """
    if not processes:raise NativeInputDenied('native app scope unavailable')
    if expected is not None:
        # MP-11: recheck the bound native leaf/ancestors before every press,
        # without repeatedly traversing unrelated/background applications.
        # X focus/geometry is independently fenced by the keyboard helper.
        process=next((item for item in processes if item['pid']==expected['pid'] and item['started']==expected['started']),None)
        if not process or not alive(process):raise NativeInputDenied('native app changed during input')
        desktop=pyatspi.Registry.getDesktop(0)
        apps=[desktop.getChildAtIndex(i) for i in range(min(desktop.childCount,64))]
        apps=[app for app in apps if app and app.get_process_id()==expected['pid']]
        if len(apps)!=1 or not 0<len(expected['path'])<=MAX_DEPTH:raise NativeInputDenied('native app ambiguous')
        node=apps[0]
        for index in expected['path']:
            if node.getRole()==pyatspi.ROLE_PASSWORD_TEXT or node.getRoleName()=='document web':raise NativeInputDenied('protected native ancestor')
            node=node.getChildAtIndex(index)
            if node is None:raise NativeInputDenied('native focused leaf changed')
        if node.getRole()==pyatspi.ROLE_PASSWORD_TEXT or node.getRoleName()=='document web':raise NativeInputDenied('protected native focus')
        state=node.getState()
        rect=node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        if not state.contains(pyatspi.STATE_FOCUSED) or not state.contains(pyatspi.STATE_SHOWING) or [rect.x,rect.y,rect.width,rect.height]!=expected['bounds']:
            raise NativeInputDenied('native focus changed during input')
        return expected
    tree=snapshot(processes)
    active=tree.get('active_window')
    # Unattributed windows (e.g. a dock without _NET_WM_PID) stay masked but
    # cannot receive keys: X focus is fenced to the proved active owned frame.
    if not tree['available'] or not tree.get('traversed') or not active:
        raise NativeInputDenied('native focus protection unavailable')
    belongs=lambda node: (node['pid']==active['pid'] and node['started']==active['started'] and
                          node['path'][:len(active['path'])]==active['path'])
    focused=[node for node in tree['nodes'] if belongs(node) and 'focused' in node['states']]
    if not focused:raise NativeInputDenied('native focused leaf unavailable')
    depth=max(len(node['path']) for node in focused)
    leaves=[node for node in focused if len(node['path'])==depth]
    if len(leaves)!=1:raise NativeInputDenied('native focus ambiguous')
    leaf=leaves[0]
    ancestors=[node for node in tree['nodes'] if belongs(node) and leaf['path'][:len(node['path'])]==node['path']]
    if any(node.get('native_protected',node['protected']) or node['role'] in ('document web','password text') for node in ancestors):
        raise NativeInputDenied('protected target requires Browser or Vault input')
    identity={key:leaf[key] for key in ('pid','started','path','bounds')}
    return identity


def input_guard(processes):
    """MP-11: bind the admitted leaf once and recheck it before every press."""
    expected = input_target(processes)
    return lambda: input_target(processes, expected)


def act(request):
    tree=snapshot(request['processes'],request.get('browser_processes'))
    if not tree['available'] or tree_digest(tree)!=request['expected_tree_digest']:raise ValueError('stale target')
    expected=next((node for node in tree['nodes'] if node['pid']==request['pid'] and node['path']==request['path'] and node['started']==request['started']),None)
    if not expected or expected['protected'] or request['action'] not in expected['actions']:raise ValueError('target denied')
    desktop=pyatspi.Registry.getDesktop(0)
    app=next((desktop.getChildAtIndex(i) for i in range(desktop.childCount) if desktop.getChildAtIndex(i).get_process_id()==request['pid']),None)
    if app is None or not alive({'pid':request['pid'],'started':request['started']}):raise ValueError('stale app')
    node=app
    for index in request['path']:node=node.getChildAtIndex(index)
    if node.getRole()==pyatspi.ROLE_PASSWORD_TEXT or node.getRoleName()!=expected['role'] or node.name!=expected['name']:raise ValueError('stale control')
    interface=node.queryAction()
    index=next((i for i in range(min(interface.nActions,16)) if interface.getName(i)==request['action']),None)
    if request.get('agent_input'):
        # MP-11: doAction can invoke Paste without naming that effect.
        import importlib.util
        spec=importlib.util.spec_from_file_location('native_clipboard',Path(__file__).with_name('native-clipboard.py'))
        clipboard=importlib.util.module_from_spec(spec);spec.loader.exec_module(clipboard)
        protection=SimpleNamespace(snapshot=snapshot,alive=alive,NativeInputDenied=NativeInputDenied)
        clipboard.input_admission(request['processes'],protection,request.get('browser_processes'))()
    if index is None or not interface.doAction(index):raise ValueError('action refused')
    return {'applied':True}
