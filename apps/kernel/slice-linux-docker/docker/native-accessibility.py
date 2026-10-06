"""MP-08 / MP-11: private-session AT-SPI tree; only verified owned applications."""
import hashlib
import json
import os
import pyatspi
MAX_NODES=512
MAX_DEPTH=32


def alive(process):
    try:
        stat=open('/proc/'+str(process['pid'])+'/stat').read()
        return (stat[stat.rfind(')')+2:].split()[19]==process['started'] and
                os.stat('/proc/'+str(process['pid'])).st_uid==os.getuid())
    except (OSError,KeyError,IndexError):return False


def snapshot(processes):
    allowed={item['pid']:item['started'] for item in processes if alive(item)}
    nodes=[];complete=True;protected=False;seen=set()
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
            for i in range(min(node.childCount,MAX_NODES)):
                child=node.getChildAtIndex(i)
                if child:visit(child,pid,started,path+[i],depth+1)
            if node.childCount>MAX_NODES:complete=False
    try:
        for app_index in range(min(desktop.childCount,64)):
            app=desktop.getChildAtIndex(app_index)
            if not app:continue
            pid=app.get_process_id()
            if pid not in allowed:
                complete=False
                continue
            seen.add(pid)
            visit(app,pid,allowed[pid],[],0)
        if desktop.childCount>64:complete=False
        # A private bus alone does not prove all visible windows expose AT-SPI.
        # Unknown/unscoped native windows make password coverage uncertain.
        from Xlib import X, display
        connection=display.Display()
        try:
            root=connection.screen().root
            clients=root.get_full_property(connection.intern_atom('_NET_CLIENT_LIST'),X.AnyPropertyType)
            for window_id in clients.value if clients is not None else []:
                window=connection.create_resource_object('window',int(window_id))
                if window.get_attributes().map_state!=X.IsViewable:continue
                pid=window.get_full_property(connection.intern_atom('_NET_WM_PID'),X.AnyPropertyType)
                if pid is None or not len(pid.value) or int(pid.value[0]) not in seen:complete=False
        finally:connection.close()
        return {'available':True,'complete':complete,'nodes':nodes,'protected':protected}
    except Exception:
        # Partial traversal cannot establish native password/pixel coverage.
        return {'available':False,'complete':False,'nodes':[],'protected':True}


def tree_digest(tree):
    return hashlib.sha256(json.dumps(tree,sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()).hexdigest()


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
