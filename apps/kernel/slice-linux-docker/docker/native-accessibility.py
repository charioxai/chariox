"""MP-08 / MP-11: private-session AT-SPI tree; only verified owned applications."""
import hashlib
import json
import os
import pyatspi
from collections import deque
# MP-08 / MP-10 / MP-11: private protection coverage is independent of
# the 64-node / 3 KiB public projection. Exhaustion still masks captures.
MAX_NODES=8192
MAX_DEPTH=32


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


def snapshot(processes):
    allowed={item['pid']:item['started'] for item in processes if alive(item)}
    nodes=[];complete=True;protected=False;seen=set();pending=deque();uncovered=[];masks=[]
    try: desktop=pyatspi.Registry.getDesktop(0)
    except Exception:return {'available':False,'complete':False,'nodes':[],'protected':True}
    def visit(node,pid,started,path,depth):
        nonlocal complete,protected
        if depth>MAX_DEPTH or len(nodes)>=MAX_NODES:complete=False;return
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
        nodes.append({'pid':pid,'started':started,'path':path,'role':role,'name':'[protected]' if secret else (node.name or '')[:4096],'states':states,'bounds':bounds,'actions':actions,'protected':secret})
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
                        complete=False
                        break
                    if child:pending.append((child,pid,started,path+[i],depth+1))
                if not managed_table and node.childCount>MAX_NODES:complete=False
            except (ValueError, NotImplementedError):
                complete=False
    try:
        for app_index in range(min(desktop.childCount,64)):
            app=desktop.getChildAtIndex(app_index)
            if not app:continue
            pid=app.get_process_id()
            if pid not in allowed:
                complete=False
                continue
            seen.add(pid)
            pending.append((app,pid,allowed[pid],[],0))
        if desktop.childCount>64:complete=False
        # MP-08: traverse applications fairly before deep hidden menu trees.
        while pending and len(nodes)<MAX_NODES:
            visit(*pending.popleft())
        if pending:complete=False
        # A private bus alone does not prove all visible windows expose AT-SPI.
        # Unknown/unscoped native windows make password coverage uncertain.
        try:
            from Xlib import X, display
        except ModuleNotFoundError as error:
            if error.name != 'Xlib': raise
            from selkies.Xlib import X, display
        connection=display.Display()
        active_window=None
        try:
            root=connection.screen().root
            active=root.get_full_property(connection.intern_atom('_NET_ACTIVE_WINDOW'),X.AnyPropertyType)
            active_id=int(active.value[0]) if active is not None and len(active.value) else None
            # MP-08: bottom-to-top order lets capture skip parts hidden by owned windows above.
            clients=root.get_full_property(connection.intern_atom('_NET_CLIENT_LIST_STACKING'),X.AnyPropertyType)
            stacked=clients is not None
            if not stacked:clients=root.get_full_property(connection.intern_atom('_NET_CLIENT_LIST'),X.AnyPropertyType)
            for window_id in clients.value if clients is not None else []:
                window=connection.create_resource_object('window',int(window_id))
                if window.get_attributes().map_state!=X.IsViewable:continue
                pid=window.get_full_property(connection.intern_atom('_NET_WM_PID'),X.AnyPropertyType)
                pid=int(pid.value[0]) if pid is not None and len(pid.value) else None
                # MP-08: an owned window without AT-SPI (e.g. xterm) has unknown
                # contents; capture blacks out its frame instead of the desktop.
                if pid in allowed and pid not in seen:
                    uncovered.append(frame_rect(root,window));masks.append(uncovered[-1])
                elif pid not in seen:complete=False
                else:
                    if stacked and masks:
                        cover=frame_rect(root,window)
                        masks=[part for rect in masks for part in subtract(rect,cover)]
                    if int(window_id)==active_id:
                        title=window.get_full_property(connection.intern_atom('_NET_WM_NAME'),X.AnyPropertyType)
                        name=bytes(title.value).decode('utf-8',errors='replace') if title is not None else window.get_wm_name()
                        frames=[node for node in nodes if node['pid']==pid and node['role'] in ('frame','window','dialog') and node['name']==name]
                        if len(frames)==1:
                            frame=frames[0]
                            active_window={key:frame[key] for key in ('pid','started','path')}
            if uncovered:
                # MP-08 / MP-11: menus, completion lists and tooltips are
                # override-redirect root children outside _NET_CLIENT_LIST with no
                # reliable owner. Mask every viewable one (over-masking is accepted).
                for child in root.query_tree().children:
                    attributes=child.get_attributes()
                    if attributes.override_redirect and attributes.map_state==X.IsViewable:
                        geometry=child.get_geometry()
                        uncovered.append([geometry.x,geometry.y,geometry.width+2*geometry.border_width,geometry.height+2*geometry.border_width]);masks.append(uncovered[-1])
        finally:connection.close()
        return {'available':True,'complete':complete,'nodes':nodes,'protected':protected,'active_window':active_window,'uncovered':uncovered,'masks':masks}
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
    tree=snapshot(processes)
    active=tree.get('active_window')
    if not tree['available'] or not tree['complete'] or not active:
        raise ValueError('native focus protection unavailable')
    belongs=lambda node: (node['pid']==active['pid'] and node['started']==active['started'] and
                          node['path'][:len(active['path'])]==active['path'])
    focused=[node for node in tree['nodes'] if belongs(node) and 'focused' in node['states']]
    if not focused:raise ValueError('native focused leaf unavailable')
    depth=max(len(node['path']) for node in focused)
    leaves=[node for node in focused if len(node['path'])==depth]
    if len(leaves)!=1:raise ValueError('native focus ambiguous')
    leaf=leaves[0]
    ancestors=[node for node in tree['nodes'] if belongs(node) and leaf['path'][:len(node['path'])]==node['path']]
    if any(node['protected'] or node['role'] in ('document web','password text') for node in ancestors):
        raise ValueError('protected target requires Browser or Vault input')
    identity={key:leaf[key] for key in ('pid','started','path','bounds')}
    if expected is not None and identity!=expected:raise ValueError('native focus changed during input')
    return identity


def act(request):
    tree=snapshot(request['processes'])
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
    if index is None or not interface.doAction(index):raise ValueError('action refused')
    return {'applied':True}
