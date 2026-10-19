"""MP-08 / MP-10 / MP-11: fair, bounded native protection coverage."""
import importlib.util
import pathlib
import sys
import types
import unittest
from unittest.mock import patch


class Node:
    def __init__(self, name, role, children=(), pid=200, secret=False):
        self.name, self.role, self.children, self.pid = name, role, children, pid
        self.childCount, self.secret = len(children), secret

    def getChildAtIndex(self, index): return self.children[index]
    def get_process_id(self): return self.pid
    def getRoleName(self): return self.role
    def getRole(self): return int(self.secret)
    def getState(self): return types.SimpleNamespace(contains=lambda flag: self.role in ('frame','window','dialog') and flag==1)
    def queryComponent(self):
        if self.role not in ('frame','window','dialog'):raise NotImplementedError
        rect=getattr(self,'rect',types.SimpleNamespace(x=100,y=80,width=300,height=200))
        return types.SimpleNamespace(getExtents=lambda coords:rect)
    def queryAction(self): raise NotImplementedError
    def queryCollection(self): raise NotImplementedError


class Cell(Node):
    def __init__(self, index, children=()):
        super().__init__('Cell '+str(index), 'table cell', children)
        self.index = index
        self.rect = types.SimpleNamespace(x=(index % 2)*10, y=(index // 2)*10, width=10, height=10)
    def getRole(self): return 3
    def getState(self): return types.SimpleNamespace(contains=lambda flag: flag == 1)
    def getIndexInParent(self): return self.index
    def queryComponent(self): return types.SimpleNamespace(getExtents=lambda coords: self.rect)


class VirtualTable(Node):
    def __init__(self, cells):
        super().__init__('Sheet', 'table', cells)
        self.childCount = 2147483647
        self.gap = False
        self.changed = False
    def getRole(self): return 2
    def getState(self): return types.SimpleNamespace(contains=lambda flag: True)
    def getChildAtIndex(self, index):
        if self.changed: return Node('Secret', 'password text', secret=True)
        return self.children[index]
    def queryComponent(self):
        return types.SimpleNamespace(
            getExtents=lambda coords: types.SimpleNamespace(x=0,y=0,width=20,height=20),
            getAccessibleAtPoint=lambda x,y,coords: None if self.gap else self.children[(y//10)*2+x//10])


class Label(Node):
    """Showing native text whose characters sit 7px apart from x=10, y=20."""
    def __init__(self, text, name='', readable=True):
        super().__init__(name, 'label')
        self.text, self.readable = text, readable
    def getState(self): return types.SimpleNamespace(contains=lambda flag: flag == 1)
    def queryComponent(self): return types.SimpleNamespace(getExtents=lambda coords: types.SimpleNamespace(x=10,y=20,width=7*len(self.text),height=14))
    def queryText(self):
        if not self.readable: raise NotImplementedError
        return types.SimpleNamespace(characterCount=len(self.text), getText=lambda start, end: self.text[start:end],
            # pyatspi.Text returns a four-item list, unlike Component extents.
            getRangeExtents=lambda start, end, coords: [10+7*start,20,7*(end-start),14])


class TraversalTest(unittest.TestCase):
    def setUp(self):
        self.desktop = Node('Desktop', 'desktop')
        atspi = types.ModuleType('pyatspi')
        atspi.Registry = types.SimpleNamespace(getDesktop=lambda index: self.desktop)
        for name in ['ROLE_PASSWORD_TEXT', 'ROLE_TABLE', 'ROLE_TABLE_CELL', 'STATE_MANAGES_DESCENDANTS', 'STATE_SHOWING', 'STATE_ENABLED', 'STATE_FOCUSED', 'STATE_EDITABLE', 'DESKTOP_COORDS']:
            setattr(atspi, name, 1)
        atspi.ROLE_TABLE = 2
        atspi.ROLE_TABLE_CELL = 3
        atspi.STATE_MANAGES_DESCENDANTS = 2
        self.connection = types.SimpleNamespace(
            screen=lambda: types.SimpleNamespace(width_in_pixels=1280,height_in_pixels=800,root=types.SimpleNamespace(get_full_property=lambda *args: None,query_tree=lambda:types.SimpleNamespace(children=[]))),
            intern_atom=lambda value: value, close=lambda: None)
        xlib = types.ModuleType('Xlib')
        xlib.X = types.SimpleNamespace(AnyPropertyType=0, IsViewable=2)
        xlib.display = types.SimpleNamespace(Display=lambda: self.connection)
        self.saved = {name: sys.modules.get(name) for name in ['pyatspi', 'Xlib']}
        sys.modules.update(pyatspi=atspi, Xlib=xlib)
        spec = importlib.util.spec_from_file_location('native_accessibility', pathlib.Path(__file__).with_name('native-accessibility.py'))
        self.driver = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.driver)
        self.driver.alive = lambda process: True
        # MP-11: synthetic trees have no fill registrations; never read the
        # launching account's real runtime ledger in a source fixture.
        fills=patch.object(self.driver._fills,'read',return_value=[])
        fills.start();self.addCleanup(fills.stop)

    def tearDown(self):
        for name, module in self.saved.items():
            if module is None: sys.modules.pop(name, None)
            else: sys.modules[name] = module

    def test_mp11_action_digest_omits_selection_text_but_keeps_identity_and_protection(self):
        base={'available':True,'nodes':[{'pid':200,'path':[0],'protected':False,'text':'first'}]}
        edited={**base,'nodes':[{**base['nodes'][0],'text':'edited'}]}
        self.assertEqual(self.driver.tree_digest(base),self.driver.tree_digest(edited))
        for change in [{'protected':True},{'path':[1]},{'pid':201}]:
            self.assertNotEqual(self.driver.tree_digest(base),self.driver.tree_digest({**edited,'nodes':[{**edited['nodes'][0],**change}]}))

    def test_mp08_public_editable_focus_is_distinct_from_submit_and_stays_target_bound(self):
        from unittest.mock import Mock
        focus=Mock(return_value=True);activate=Mock(return_value=True)
        node=Node('Username','text')
        node.getState=lambda:types.SimpleNamespace(contains=lambda flag:True)
        rect=types.SimpleNamespace(x=110,y=100,width=200,height=30)
        node.queryComponent=lambda:types.SimpleNamespace(getExtents=lambda _:rect,grabFocus=focus)
        node.queryAction=lambda:types.SimpleNamespace(nActions=1,getName=lambda _:'activate',doAction=activate)
        self.desktop=Node('Desktop','desktop',[Node('Office','application',[Node('Writer','frame',[node])])])
        processes=[{'pid':200,'started':'1'}]
        tree=self.driver.snapshot(processes);entry=next(n for n in tree['nodes'] if n['name']=='Username')
        self.assertEqual(entry['actions'],['focus','activate'])
        request={'processes':processes,'expected_tree_digest':self.driver.tree_digest(tree),'pid':200,'started':'1','path':entry['path'],'action':'focus'}
        self.assertEqual(self.driver.act(request),{'applied':True})
        focus.assert_called_once();activate.assert_not_called()
        node.name='Different field'
        with self.assertRaises(ValueError):self.driver.act(request)
        focus.assert_called_once()
        class NoSecretReads(Node):
            def __getattribute__(self,key):
                if key=='name' and object.__getattribute__(self,'secret'):raise AssertionError('private field name queried')
                return super().__getattribute__(key)
        node.__class__=NoSecretReads
        node.queryText=lambda:(_ for _ in ()).throw(AssertionError('private field text queried'))
        node.secret=True;node.role='password text'
        protected=self.driver.snapshot(processes);entry=next(n for n in protected['nodes'] if n['role']=='password text')
        self.assertEqual(entry['actions'],['focus'])
        self.assertEqual(self.driver.act({**request,'expected_tree_digest':self.driver.tree_digest(protected)}),{'applied':True})
        self.assertEqual(focus.call_count,2)
        with self.assertRaises(ValueError):self.driver.act({**request,'action':'activate','expected_tree_digest':self.driver.tree_digest(protected)})
        activate.assert_not_called()

    def test_mp11_public_selection_evidence_never_queries_protected_or_long_text(self):
        for protected,length,readable in [(False,12,True),(False,4097,False),(True,12,False)]:
            with self.subTest(protected=protected,length=length):
                node=Node('Entry','password text' if protected else 'text',secret=protected)
                node.getState=lambda:types.SimpleNamespace(contains=lambda flag:True)
                reads=[]
                node.queryText=lambda:types.SimpleNamespace(characterCount=length,getText=lambda *args:(reads.append(args) or 'public value'))
                self.desktop=Node('Desktop','desktop',[Node('Office','application',[Node('Writer','frame',[node])])])
                tree=self.driver.snapshot([{'pid':200,'started':'1'}])
                entry=next(n for n in tree['nodes'] if n['role']==node.role)
                self.assertEqual('text' in entry,readable)
                self.assertEqual(bool(reads),readable)
                if protected:self.assertEqual(entry['name'],'[protected]')

    def test_mp11_capture_ignores_echoes_unknown_windows_and_password_dots(self):
        self.desktop = Node('Desktop', 'desktop', [Node('Writer', 'application', [
            Label('v-secret'), Node('Password', 'password text', secret=True)])])
        with patch.object(self.driver._fills, 'regions', return_value=[]):
            result = self.driver.capture_snapshot([{'pid': 200, 'started': '1'}], values=['v-secret'])
        self.assertEqual(result['masks'], [])
        self.assertTrue(result['available'] and result['complete'])

    def test_mp11_capture_uses_only_live_plain_field_ledger(self):
        with patch.object(self.driver._fills, 'regions', return_value=[[10, 20, 80, 30]]):
            result = self.driver.capture_snapshot([], values=['value that also appears elsewhere'])
        self.assertEqual(result['masks'], [[10, 20, 80, 30]])
        self.assertEqual(result['nodes'], [])

    def test_mp08_mp11_scaled_frame_geometry_does_not_turn_echoes_into_masks(self):
        self.foreground()
        text=Label('v-secret',name='Plain text')
        text.queryText=lambda:types.SimpleNamespace(characterCount=8,getText=lambda *args:'v-secret',
            getRangeExtents=lambda *args:[55,65,56,14])
        frame=Node('Writer','frame',[text]);frame.rect=types.SimpleNamespace(x=50,y=40,width=150,height=100)
        self.desktop=Node('Desktop','desktop',[Node('Office','application',[frame])])
        tree=self.driver.snapshot([{'pid':200,'started':'1'}],values=['v-secret'])
        self.assertEqual(tree['uncovered'],[])
        self.assertEqual(tree['nodes'][-1]['bounds'],[10,20,56,14])
        self.assertEqual(tree['nodes'][-1]['desktop_bounds'],[20,40,112,28])
        self.assertEqual(tree['masks'],[])
        self.assertEqual(self.driver.native_frame_scale([50,40,150,100],[100,80,300,200],[100,80,300,200]),2)
        self.assertIsNone(self.driver.native_frame_scale([50,40,150,100],[100,80,300,240],[100,80,300,240]))
        self.assertIsNone(self.driver.native_frame_scale([50,40,150,100],[150,120,450,300],[150,120,450,300]))

    def test_mp08_mp11_top_level_gtk_file_chooser_keeps_exact_window_focus_proof(self):
        # Real Mousepad Save As exposes its top-level window as file chooser.
        # The title, geometry, live PID and unique window proof still apply.
        self.foreground()
        leaf=Node('Filename','text')
        leaf.getState=lambda:types.SimpleNamespace(contains=lambda flag:True)
        leaf.queryComponent=lambda:types.SimpleNamespace(getExtents=lambda coords:types.SimpleNamespace(x=110,y=100,width=200,height=30))
        chooser=Node('Writer','file chooser',[leaf])
        chooser.getState=lambda:types.SimpleNamespace(contains=lambda flag:True)
        chooser.rect=types.SimpleNamespace(x=100,y=80,width=300,height=200)
        chooser.queryComponent=lambda:types.SimpleNamespace(getExtents=lambda coords:chooser.rect)
        self.desktop=Node('Desktop','desktop',[Node('Office','application',[chooser])])
        processes=[{'pid':200,'started':'1'}]
        self.assertEqual(self.driver.input_target(processes)['path'],[0,0])
        self.driver.input_guard(processes)()
        for change in ['title','geometry','nested','ambiguous']:
            with self.subTest(change=change):
                chooser.name='Writer';chooser.rect.width=300;leaf.secret=False;leaf.role='text'
                roots=[chooser]
                if change=='title':chooser.name='Other'
                if change=='geometry':chooser.rect.width=301
                if change=='nested':roots=[Node('Container','panel',[chooser])]
                if change=='ambiguous':roots=[chooser,chooser]
                if change=='password':leaf.secret=True
                if change=='browser':leaf.role='document web'
                self.desktop=Node('Desktop','desktop',[Node('Office','application',roots)])
                with self.assertRaises(self.driver.NativeInputDenied):self.driver.input_target(processes)

    def test_mp11_unattributed_dock_does_not_block_owned_focus_input(self):
        # MP-11 (#904 Room drill): tint2 maps a dock without _NET_WM_PID. It stays
        # masked and the capture tree incomplete, but keys go to the proved frame.
        self.foreground()
        root=self.connection.screen().root;writer=self.connection.create_resource_object('window',9)
        dock=types.SimpleNamespace(id=10,get_attributes=lambda:types.SimpleNamespace(map_state=2),get_full_property=lambda atom,kind:None,
            get_wm_name=lambda:'tint2',get_geometry=lambda:types.SimpleNamespace(x=0,y=770,width=1280,height=30,border_width=0),query_tree=lambda:types.SimpleNamespace(parent=root))
        root.get_full_property=lambda atom,kind:types.SimpleNamespace(value=[9,10] if atom=='_NET_CLIENT_LIST_STACKING' else [9])
        self.connection.create_resource_object=lambda kind,value:writer if value==9 else dock
        class Focused(Node):
            def getState(self):return types.SimpleNamespace(contains=lambda flag:True)
        apps=[Node('Office','application',[Node('Writer','frame',[Focused('Body','text')])])]
        tree=self.snapshot(apps)
        self.assertFalse(tree['complete'])
        self.assertIn([0,770,1280,30],tree['uncovered'])
        self.desktop=Node('Desktop','desktop',apps)
        self.assertEqual(self.driver.input_target([{'pid':200,'started':'1'}])['path'],[0,0])
        self.assertTrue(tree['traversed'])

    def test_mp11_review3_owned_focus_is_independent_of_stacking_order_and_unowned_apps(self):
        # #904 review 3: an unattributed window earlier in stacking order, or an
        # unowned AT-SPI app, made capture incomplete and hid the proved frame.
        self.foreground()
        root=self.connection.screen().root;writer=self.connection.create_resource_object('window',9)
        dock=types.SimpleNamespace(id=10,get_attributes=lambda:types.SimpleNamespace(map_state=2),get_full_property=lambda atom,kind:None,
            get_wm_name=lambda:'tint2',get_geometry=lambda:types.SimpleNamespace(x=0,y=770,width=1280,height=30,border_width=0),query_tree=lambda:types.SimpleNamespace(parent=root))
        self.connection.create_resource_object=lambda kind,value:writer if value==9 else dock
        class Focused(Node):
            def getState(self):return types.SimpleNamespace(contains=lambda flag:True)
        owned=Node('Office','application',[Node('Writer','frame',[Focused('Body','text')])])
        for stacking,apps in [([10,9],[owned]),([9,10],[Node('Foreign','application',[Node('Other','frame')],pid=999),owned])]:
            root.get_full_property=lambda atom,kind,stacking=stacking:types.SimpleNamespace(value=stacking if atom=='_NET_CLIENT_LIST_STACKING' else [9])
            tree=self.snapshot(apps)
            self.assertFalse(tree['complete'],stacking)
            self.assertIn([0,770,1280,30],tree['uncovered'])
            self.assertEqual(tree['active_window'],{'pid':200,'started':'1','path':[0]},stacking)
            self.desktop=Node('Desktop','desktop',apps)
            self.assertEqual(self.driver.input_target([{'pid':200,'started':'1'}])['path'],[0,0])
        # The actual unowned foreground (the dock) is still refused.
        root.get_full_property=lambda atom,kind:types.SimpleNamespace(value=[9,10] if atom=='_NET_CLIENT_LIST_STACKING' else [10])
        self.desktop=Node('Desktop','desktop',[owned])
        with self.assertRaises(self.driver.NativeInputDenied):self.driver.input_target([{'pid':200,'started':'1'}])

    def test_mp11_registered_native_fill_stays_private_when_its_field_is_plain(self):
        field=Node('private-field-value','text');field.path='/field'
        app=Node('Login','application',[Node('Login','frame',[field,Node('Sign in','push button')])])
        with patch.object(self.driver._fills,'read',return_value=[{'pid':200,'started':'1','path':'/field'}]), patch.object(self.driver._fills,'matches',return_value=True):
            tree=self.snapshot([app])
        secret=next(node for node in tree['nodes'] if node['role']=='text')
        self.assertTrue(secret['protected']);self.assertEqual(secret['name'],'[protected]')
        self.assertTrue(any(node['name']=='Sign in' and not node['protected'] for node in tree['nodes']))
        self.assertTrue(any(node['role']=='frame' and node['name']=='Login' for node in tree['nodes']))

    def test_mp08_mp11_registered_values_do_not_scan_native_text(self):
        self.desktop = Node('Desktop', 'desktop', [Node('Writer', 'application', [Node('Writer', 'frame', [Label('v-secret'), Label('ordinary')])])])
        tree = self.driver.snapshot([{'pid': 200, 'started': '1'}], values=['v-secret'])
        self.assertTrue(tree['available'])
        self.assertEqual(tree['masks'], [])

    def snapshot(self, applications):
        self.desktop = Node('Desktop', 'desktop', applications)
        return self.driver.snapshot([{'pid': 200, 'started': '1'}, {'pid': 201, 'started': '2'}])

    def test_mp11_unresolved_child_blocks_clipboard_paste_and_live_press_fence(self):
        # Exercise the producer, not a fabricated traversed=False receipt.
        from unittest.mock import Mock
        spec = importlib.util.spec_from_file_location('native_clipboard', pathlib.Path(__file__).with_name('native-clipboard.py'))
        clipboard = importlib.util.module_from_spec(spec); spec.loader.exec_module(clipboard)
        pressed = Mock()
        action = types.SimpleNamespace(nActions=1, getName=lambda i: 'paste', doAction=pressed)
        paste = Node('Paste', 'push button'); paste.queryAction = lambda: action
        app = Node('Editor', 'application', [paste])
        self.desktop = Node('Desktop', 'desktop', [app])
        owner = types.SimpleNamespace(id=99, get_full_property=lambda *args: None)
        self.connection.get_selection_owner = lambda atom: owner
        self.connection.has_extension = lambda name: True
        self.connection.res_query_version = lambda: types.SimpleNamespace(server_major=1, server_minor=2)
        self.connection.res_query_client_ids = lambda ids: types.SimpleNamespace(ids=[types.SimpleNamespace(spec=types.SimpleNamespace(mask=2), value=[200])])
        processes = [{'pid': 200, 'started': '1'}]
        with patch.object(clipboard, 'selection_fingerprint', return_value='public-selection'):
            fence = clipboard.input_admission(processes, self.driver)
            fence()  # A completely resolved public owner is admitted.
            app.children.append(None); app.childCount += 1
            tree = self.driver.snapshot(processes)
            for admit in (lambda: clipboard.input_admission(processes, self.driver), fence):
                with self.assertRaises(self.driver.NativeInputDenied):
                    admit(); pressed()  # Pointer/key event must not be dispatched.
        with self.assertRaises(self.driver.NativeInputDenied):
            self.driver.act({'processes': processes, 'pid': 200, 'started': '1', 'path': [0],
                             'action': 'paste', 'agent_input': True,
                             'expected_tree_digest': self.driver.tree_digest(tree)})
        pressed.assert_not_called()
        self.assertFalse(tree['complete'])
        self.assertFalse(tree['traversed'])

    def foreground(self, pid=200, name='Writer'):
        window = types.SimpleNamespace(get_attributes=lambda: types.SimpleNamespace(map_state=2),
            get_full_property=lambda atom, kind: types.SimpleNamespace(value=[pid] if atom=='_NET_WM_PID' else name.encode()))
        root = types.SimpleNamespace(id=1,get_full_property=lambda atom, kind: types.SimpleNamespace(value=[9]),query_tree=lambda:types.SimpleNamespace(children=[]),translate_coords=lambda *args:types.SimpleNamespace(x=100,y=80))
        window.id=9
        window.query_tree=lambda:types.SimpleNamespace(parent=root)
        window.get_geometry=lambda:types.SimpleNamespace(x=100,y=80,width=300,height=200,border_width=0)
        self.connection.screen = lambda: types.SimpleNamespace(width_in_pixels=1280,height_in_pixels=800,root=root)
        self.connection.create_resource_object = lambda kind, value: window

    def test_mp08_foreground_binds_exact_owned_frame(self):
        self.foreground()
        tree=self.snapshot([Node('Office','application',[Node('Writer','frame')])])
        self.assertEqual(tree['active_window'],{'pid':200,'started':'1','path':[0]})

    def test_mp11_foreign_foreground_does_not_select_owned_frame(self):
        self.foreground(pid=999)
        tree=self.snapshot([Node('Office','application',[Node('Writer','frame')])])
        self.assertIsNone(tree['active_window'])
        self.assertFalse(tree['complete'])

    def test_mp11_ambiguous_foreground_does_not_select_a_frame(self):
        self.foreground()
        tree=self.snapshot([Node('Office','application',[Node('Writer','frame'),Node('Writer','frame')])])
        self.assertIsNone(tree['active_window'])

    def test_mp08_mp11_kernel_browser_window_reveals_all_but_proven_protected_regions(self):
        self.foreground(pid=200, name='Chromium')
        class Document(Node):
            def queryComponent(self):
                return types.SimpleNamespace(getExtents=lambda coords: types.SimpleNamespace(x=100, y=120, width=300, height=160))
            def queryDocument(self):
                return types.SimpleNamespace(getAttributeValue=lambda key: 'https://example.test/' if key == 'URI' else '')
        self.desktop = Node('Desktop', 'desktop', [Node('Chromium', 'application', [Node('Chromium', 'frame', [Document('Page', 'document web')])])])
        page = {'url': 'https://example.test/', 'window': [100, 80, 300, 200], 'viewport': [300, 160], 'dpr': 1, 'zoom': 1, 'regions': [[10, 10, 20, 20]], 'chrome': False}
        snapshot = lambda protection: self.driver.snapshot([{'pid': 200, 'started': '1'}], [{'pid': 200, 'started': '1'}], protection)
        self.assertEqual(snapshot({'pages': [page]})['masks'], [[106, 126, 28, 28]])
        self.assertEqual(snapshot({'pages': [page]})['browser_withheld'], 0)
        # Unmeasured, moved/resized or navigated windows have no fill placement.
        for protection in [None, {'pages': [{**page, 'window': [101, 80, 300, 200]}]}, {'pages': [{**page, 'url': 'https://other.test/'}]}]:
            self.assertEqual(snapshot(protection)['masks'], [])
            self.assertEqual(snapshot(protection)['browser_withheld'], 0 if protection is None else 1)
        # MP-08/MP-11: harmless Chromium navigation remains available.
        self.assertIn('Page', [node['name'] for node in snapshot({'pages': [page]})['nodes']])

    def test_mp08_other_window_before_deep_hidden_menu_budget(self):
        self.driver.MAX_NODES = 512
        menus = [Node('Hidden menu', 'menu item') for _ in range(512)]
        editor = Node('Editor', 'application', [Node('Editor window', 'frame', [
            Node('Menu bar', 'menu bar', menus)])])
        canvas = Node('Canvas', 'application', [Node('Chariox pixel fixture', 'frame', pid=201)], pid=201)
        tree = self.snapshot([editor, canvas])
        self.assertTrue(tree['available'])
        self.assertFalse(tree['complete'])
        self.assertLessEqual(len(tree['nodes']), 512)
        node = next(node for node in tree['nodes'] if node['name'] == 'Chariox pixel fixture')
        self.assertEqual((node['pid'], node['started'], node['path']), (201, '2', [0]))

    def test_mp11_secret_after_public_projection_budget_remains_protected(self):
        controls = [Node('Public', 'label') for _ in range(1934)]
        controls.append(Node('Never expose this password', 'password text', secret=True))
        tree = self.snapshot([Node('Writer', 'application', controls)])
        self.assertTrue(tree['complete'])
        self.assertTrue(tree['protected'])
        self.assertEqual(tree['nodes'][-1]['name'], '[protected]')
        self.assertEqual(tree['nodes'][-1]['actions'], [])

    def test_mp11_larger_private_budget_exhaustion_still_fails_closed(self):
        controls = [Node('Public', 'label') for _ in range(8192)]
        controls.append(Node('Unobserved secret', 'password text', secret=True))
        tree = self.snapshot([Node('Calc', 'application', controls)])
        self.assertTrue(tree['available'])
        self.assertFalse(tree['complete'])
        self.assertLessEqual(len(tree['nodes']), 8192)

    def test_mp08_virtual_table_covers_visible_cells_without_enumerating_billions(self):
        table = VirtualTable([Cell(i) for i in range(4)])
        tree = self.snapshot([Node('Calc', 'application', [table])])
        self.assertTrue(tree['complete'])
        self.assertEqual([n['path'] for n in tree['nodes'] if n['role']=='table cell'], [[0,i] for i in range(4)])

    def bounded_table(self, hidden, secret=False):
        children = [Node('Name', 'table column header'), Cell(1,
            [Node('Never expose', 'password text', secret=True)] if secret else [])]
        table = VirtualTable(children)
        table.childCount = len(children)
        table.gap = True  # GTK headers/empty space do not resolve to table cells.
        table.getState = lambda: types.SimpleNamespace(contains=lambda flag:
            flag == 2 or (flag == 1 and not hidden))
        if hidden:
            table.queryComponent = lambda: types.SimpleNamespace(getExtents=lambda coords:
                types.SimpleNamespace(x=-2147483648,y=-2147483648,width=1,height=1))
        return table

    def test_mp08_bounded_gtk_tables_cover_headers_and_hidden_children(self):
        for hidden in [False, True]:
            tree = self.snapshot([Node('Save', 'application', [self.bounded_table(hidden)])])
            self.assertTrue(tree['complete'])
            self.assertFalse(tree['protected'])
            self.assertEqual(len(tree['nodes']), 4)

    def test_mp11_bounded_gtk_table_password_descendants_stay_protected(self):
        for hidden in [False, True]:
            tree = self.snapshot([Node('Save', 'application', [self.bounded_table(hidden, True)])])
            self.assertTrue(tree['complete'])
            self.assertTrue(tree['protected'])
            self.assertEqual(tree['nodes'][-1]['name'], '[protected]')

    def test_mp11_virtual_cell_secret_descendants_still_protect_pixels(self):
        cells = [Cell(i) for i in range(4)]
        cells[3].children = [Node('Secret', 'password text', secret=True)]
        cells[3].childCount = 1
        tree = self.snapshot([Node('Calc', 'application', [VirtualTable(cells)])])
        self.assertTrue(tree['complete'])
        self.assertTrue(tree['protected'])

    def test_mp11_virtual_table_gaps_and_changed_indexed_targets_fail_closed(self):
        for field in ['gap', 'changed']:
            table = VirtualTable([Cell(i) for i in range(4)])
            setattr(table, field, True)
            tree = self.snapshot([Node('Calc', 'application', [table])])
            self.assertFalse(tree['complete'])


    def terminal(self, pid, popups=()):
        # An owned xterm: visible, reparented into a WM frame, without an AT-SPI app.
        root = types.SimpleNamespace(id=1, get_full_property=lambda atom, kind: types.SimpleNamespace(value=[9]))
        frame = types.SimpleNamespace(id=8, query_tree=lambda: types.SimpleNamespace(parent=root),
            get_attributes=lambda: types.SimpleNamespace(map_state=2, override_redirect=0),
            get_geometry=lambda: types.SimpleNamespace(x=20, y=30, width=244, height=150, border_width=1))
        root.query_tree = lambda: types.SimpleNamespace(children=[frame, *popups])
        root.translate_coords=lambda *args:types.SimpleNamespace(x=20,y=30)
        window = types.SimpleNamespace(id=9, get_geometry=lambda:types.SimpleNamespace(width=246,height=152),query_tree=lambda: types.SimpleNamespace(parent=frame),
            get_attributes=lambda: types.SimpleNamespace(map_state=2),
            get_full_property=lambda atom, kind: types.SimpleNamespace(value=[pid] if atom=='_NET_WM_PID' else b'xterm'))
        self.connection.screen = lambda: types.SimpleNamespace(width_in_pixels=1280,height_in_pixels=800,root=root)
        self.connection.create_resource_object = lambda kind, value: window

    def test_mp08_owned_window_without_accessibility_has_no_visual_mask(self):
        self.terminal(201)
        tree = self.snapshot([Node('Office', 'application', [Node('Writer', 'frame')])])
        self.assertTrue(tree['available'])
        self.assertTrue(tree['complete'])
        self.assertFalse(tree['protected'])
        self.assertEqual(tree['uncovered'], [[20, 30, 246, 152]])
        self.assertEqual(tree['masks'], [])

    def test_mp11_override_redirect_popups_have_no_visual_mask(self):
        # Menus, completion lists and tooltips are outside _NET_CLIENT_LIST.
        popup = lambda x, map_state: types.SimpleNamespace(
            get_attributes=lambda: types.SimpleNamespace(map_state=map_state, override_redirect=1),
            get_geometry=lambda: types.SimpleNamespace(x=x, y=40, width=100, height=60, border_width=1))
        self.terminal(201, [popup(300, 2), popup(500, 0)])
        tree = self.snapshot([Node('Office', 'application', [Node('Writer', 'frame')])])
        self.assertTrue(tree['complete'])
        self.assertFalse(tree['protected'])
        self.assertEqual(tree['uncovered'], [[20, 30, 246, 152], [300, 40, 102, 62]])
        self.assertEqual(tree['masks'], [])

    def stacked(self, order, stacking=True):
        # MP-08: owned xterm (9, no AT-SPI) and owned Writer (10) frames in WM stacking order.
        root = types.SimpleNamespace(id=1)
        def client(window_id, pid, name, x, y, width, height):
            frame = types.SimpleNamespace(id=window_id*10, query_tree=lambda: types.SimpleNamespace(parent=root),
                get_attributes=lambda: types.SimpleNamespace(map_state=2, override_redirect=0),
                get_geometry=lambda: types.SimpleNamespace(x=x, y=y, width=width, height=height, border_width=1))
            return types.SimpleNamespace(id=window_id,get_geometry=lambda:types.SimpleNamespace(width=width+2,height=height+2),query_tree=lambda: types.SimpleNamespace(parent=frame),
                get_attributes=lambda: types.SimpleNamespace(map_state=2),
                get_full_property=lambda atom, kind: types.SimpleNamespace(value=[pid] if atom=='_NET_WM_PID' else name))
        windows = {9: client(9, 201, b'xterm', 20, 30, 244, 150), 10: client(10, 200, b'Writer', 100, 80, 298, 198)}
        lists = {'_NET_ACTIVE_WINDOW': [10], '_NET_CLIENT_LIST': [9, 10]}
        if stacking: lists['_NET_CLIENT_LIST_STACKING'] = order
        root.get_full_property = lambda atom, kind: types.SimpleNamespace(value=lists[atom]) if atom in lists else None
        root.query_tree = lambda: types.SimpleNamespace(children=[])
        root.translate_coords=lambda window,*args:types.SimpleNamespace(x=20 if window.id==9 else 100,y=30 if window.id==9 else 80)
        self.connection.screen = lambda: types.SimpleNamespace(width_in_pixels=1280,height_in_pixels=800,root=root)
        self.connection.create_resource_object = lambda kind, value: windows[value]

    def test_mp08_owned_window_below_an_owned_app_has_no_visual_mask(self):
        self.stacked([9, 10])
        tree = self.snapshot([Node('Office', 'application', [Node('Writer', 'frame')])])
        self.assertTrue(tree['complete'])
        self.assertFalse(tree['protected'])
        # MP-11: unknown-content window still recorded (clipboard and completeness stay closed).
        self.assertEqual(tree['uncovered'], [[20, 30, 246, 152]])
        # Visibility does not invent a Vault fill target.
        self.assertEqual(tree['masks'], [])
        # Masking an owned window below must not unbind the focused Writer frame.
        self.assertEqual(tree['active_window'], {'pid': 200, 'started': '1', 'path': [0]})

    def test_mp11_owned_window_above_or_unknown_stacking_has_no_visual_mask(self):
        for order, stacking in [([10, 9], True), ([9, 10], False)]:
            self.stacked(order, stacking)
            tree = self.snapshot([Node('Office', 'application', [Node('Writer', 'frame')])])
            self.assertEqual(tree['masks'], [])

    def test_mp11_foreign_or_unattributed_window_denies_input_authority(self):
        for pid in [999, None]:
            self.terminal(pid)
            if pid is None:
                window = self.connection.create_resource_object('window', 9)
                window.get_full_property = lambda atom, kind: None
            tree = self.snapshot([Node('Office', 'application', [Node('Writer', 'frame')])])
            self.assertFalse(tree['complete'])

    def test_mp11_finding1_native_text_focus_is_owned_complete_and_not_protected_or_web(self):
        from unittest.mock import patch
        frame={'pid':200,'started':'1','path':[0],'role':'frame','protected':False,'states':['showing'],'bounds':[0,0,640,480]}
        leaf={**frame,'path':[0,0],'role':'text','states':['showing','focused','editable']}
        tree={'available':True,'complete':True,'traversed':True,'protected':False,'active_window':{key:frame[key] for key in ('pid','started','path')},'nodes':[frame,leaf],'uncovered':[]}
        with patch.object(self.driver,'snapshot',return_value=tree):
            expected=self.driver.input_target([{'pid':200,'started':'1'}])
            self.assertEqual(expected['path'],[0,0])
            for changed in [{**tree,'traversed':False},{**tree,'active_window':None},
                {**tree,'nodes':[frame,{**leaf,'protected':True}]},
                {**tree,'nodes':[frame,{**leaf,'role':'document web'}]},
                {**tree,'nodes':[frame,{**leaf,'path':[0,1]}]}]:
                with patch.object(self.driver,'snapshot',return_value=changed):
                    with self.assertRaises(ValueError):self.driver.input_target([],expected)

    def test_mp08_mp11_browser_navigation_exposes_labels_and_redacts_secret_fields(self):
        self.foreground(pid=200,name='Chromium')
        otp=Node('test-otp-value','text',[Node('test-otp-value','text')])
        otp.getAttributes=lambda:['tag:input','text-input-type:text','autocomplete:one-time-code']
        password=Node('test-password-value','password text',secret=True)
        vault=Node('test-vault-value','text')
        browser=Node('Chromium','application',[Node('Chromium','frame',[
            Node('Accept cookies','push button'),Node('Address and search bar','text'),
            otp,password,vault,Node('Public article','document web')])])
        self.desktop=Node('Desktop','desktop',[browser]);processes=[{'pid':200,'started':'1'}]
        with patch.object(self.driver._fills,'read',return_value=[]):
            tree=self.driver.snapshot(processes,browser_processes=processes,values=['test-vault-value'])
        names=[node['name'] for node in tree['nodes']]
        self.assertIn('Accept cookies',names);self.assertIn('Address and search bar',names)
        self.assertIn('Public article',names)
        self.assertNotIn('test-otp-value',str(tree));self.assertNotIn('test-password-value',str(tree))
        self.assertNotIn('test-vault-value',str(tree))
        self.assertTrue(tree['protected']);self.assertIsNotNone(tree['active_window'])

    def test_mp11_vault_value_is_scrubbed_before_native_label_truncation(self):
        value='test-vault-'+'x'*5000
        self.desktop=Node('Desktop','desktop',[Node('Chromium','application',[Node(value,'text')])])
        tree=self.driver.snapshot([{'pid':200,'started':'1'}],values=[value])
        self.assertNotIn('test-vault-',str(tree))
        self.assertIn('[redacted]',str(tree))

    def test_mp08_mp11_native_keyboard_accepts_owned_browser_and_secret_focus(self):
        self.foreground(pid=200,name='Chromium')
        for role,secret in [('text',False),('document web',False),('password text',True)]:
            with self.subTest(role=role):
                leaf=Node('field','%s'%role,secret=secret)
                leaf.getState=lambda:types.SimpleNamespace(contains=lambda flag:True)
                leaf.queryComponent=lambda:types.SimpleNamespace(getExtents=lambda coords:types.SimpleNamespace(x=110,y=100,width=100,height=30))
                self.desktop=Node('Desktop','desktop',[Node('Chromium','application',[Node('Chromium','frame',[leaf])])])
                processes=[{'pid':200,'started':'1'}]
                target=self.driver.input_target(processes)
                self.assertEqual(target['path'],[0,0]);self.assertEqual(self.driver.input_target(processes,target),target)

    def test_mp11_finding3_popup_has_no_visual_mask(self):
        popup=types.SimpleNamespace(get_attributes=lambda:types.SimpleNamespace(map_state=2,override_redirect=1),
            get_geometry=lambda:types.SimpleNamespace(x=300,y=40,width=100,height=60,border_width=1))
        self.terminal(200,[popup])
        frame=Node('xterm','frame');frame.rect=types.SimpleNamespace(x=20,y=30,width=246,height=152)
        tree=self.snapshot([Node('Office','application',[frame])])
        self.assertEqual(tree['masks'], [])

    def test_mp11_r3_hidden_selection_window_has_no_desktop_pixels(self):
        for bounds, expected in [((-100,-100,1,1),[]),((-5,40,15,60),[[0,40,10,60]]),((1270,790,20,20),[[1270,790,10,10]])]:
            x,y,width,height=bounds
            popup=types.SimpleNamespace(get_attributes=lambda:types.SimpleNamespace(map_state=2,override_redirect=1),
                get_geometry=lambda:types.SimpleNamespace(x=x,y=y,width=width,height=height,border_width=0))
            self.terminal(200,[popup])
            frame=Node('xterm','frame');frame.rect=types.SimpleNamespace(x=20,y=30,width=246,height=152)
            tree=self.snapshot([Node('Office','application',[frame])])
            self.assertEqual(tree['masks'], [])

    def test_mp11_finding4_same_pid_unmatched_window_has_no_coverage(self):
        self.terminal(200)
        tree=self.snapshot([Node('Office','application',[Node('Writer','frame')])])
        self.assertEqual(tree['masks'], [])
        self.assertIsNone(tree['active_window'])

    def test_mp11_finding4_ambiguous_or_wrong_geometry_denies_input_authority(self):
        for frames in [[Node('Writer','frame'),Node('Writer','frame')],[Node('Other','frame')],[]]:
            self.stacked([9,10])
            tree=self.snapshot([Node('Office','application',frames)])
            self.assertEqual(tree['masks'], [])
            self.assertIsNone(tree['active_window'])

    def test_mp11_finding1_repeated_focus_checks_use_live_ancestors_and_leaf(self):
        leaf=Node('public','text');leaf.getState=lambda:types.SimpleNamespace(contains=lambda flag:True)
        rect=types.SimpleNamespace(x=30,y=50,width=200,height=30)
        leaf.queryComponent=lambda:types.SimpleNamespace(getExtents=lambda coords:rect)
        frame=Node('Editor','frame',[leaf]);self.desktop=Node('Desktop','desktop',[Node('Editor','application',[frame])])
        expected={'pid':200,'started':'1','path':[0,0],'bounds':[30,50,200,30]}
        processes=[{'pid':200,'started':'1'}]
        self.assertEqual(self.driver.input_target(processes,expected),expected)
        frame.role='document web'
        self.assertEqual(self.driver.input_target(processes,expected),expected)
        frame.role='frame';leaf.secret=True
        self.assertEqual(self.driver.input_target(processes,expected),expected)
        leaf.secret=False;leaf.getState=lambda:types.SimpleNamespace(contains=lambda flag:False)
        with self.assertRaises(ValueError):self.driver.input_target(processes,expected)


if __name__ == '__main__': unittest.main()
