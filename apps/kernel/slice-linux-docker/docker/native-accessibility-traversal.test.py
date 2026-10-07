"""MP-08 / MP-10 / MP-11: fair, bounded native protection coverage."""
import importlib.util
import pathlib
import sys
import types
import unittest


class Node:
    def __init__(self, name, role, children=(), pid=200, secret=False):
        self.name, self.role, self.children, self.pid = name, role, children, pid
        self.childCount, self.secret = len(children), secret

    def getChildAtIndex(self, index): return self.children[index]
    def get_process_id(self): return self.pid
    def getRoleName(self): return self.role
    def getRole(self): return int(self.secret)
    def getState(self): return types.SimpleNamespace(contains=lambda flag: False)
    def queryComponent(self): raise NotImplementedError
    def queryAction(self): raise NotImplementedError


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
            screen=lambda: types.SimpleNamespace(root=types.SimpleNamespace(get_full_property=lambda *args: None)),
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

    def tearDown(self):
        for name, module in self.saved.items():
            if module is None: sys.modules.pop(name, None)
            else: sys.modules[name] = module

    def snapshot(self, applications):
        self.desktop = Node('Desktop', 'desktop', applications)
        return self.driver.snapshot([{'pid': 200, 'started': '1'}, {'pid': 201, 'started': '2'}])

    def foreground(self, pid=200, name='Writer'):
        window = types.SimpleNamespace(get_attributes=lambda: types.SimpleNamespace(map_state=2),
            get_full_property=lambda atom, kind: types.SimpleNamespace(value=[pid] if atom=='_NET_WM_PID' else name.encode()))
        root = types.SimpleNamespace(get_full_property=lambda atom, kind: types.SimpleNamespace(value=[9]))
        self.connection.screen = lambda: types.SimpleNamespace(root=root)
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


    def terminal(self, pid):
        # An owned xterm: visible, reparented into a WM frame, without an AT-SPI app.
        root = types.SimpleNamespace(id=1, get_full_property=lambda atom, kind: types.SimpleNamespace(value=[9]))
        frame = types.SimpleNamespace(id=8, query_tree=lambda: types.SimpleNamespace(parent=root),
            get_geometry=lambda: types.SimpleNamespace(x=20, y=30, width=244, height=150, border_width=1))
        window = types.SimpleNamespace(id=9, query_tree=lambda: types.SimpleNamespace(parent=frame),
            get_attributes=lambda: types.SimpleNamespace(map_state=2),
            get_full_property=lambda atom, kind: types.SimpleNamespace(value=[pid] if atom=='_NET_WM_PID' else b'xterm'))
        self.connection.screen = lambda: types.SimpleNamespace(root=root)
        self.connection.create_resource_object = lambda kind, value: window

    def test_mp08_owned_window_without_accessibility_masks_only_its_frame(self):
        self.terminal(201)
        tree = self.snapshot([Node('Office', 'application', [Node('Writer', 'frame')])])
        self.assertTrue(tree['available'])
        self.assertTrue(tree['complete'])
        self.assertFalse(tree['protected'])
        self.assertEqual(tree['uncovered'], [[20, 30, 246, 152]])

    def test_mp11_foreign_or_unattributed_window_still_masks_the_desktop(self):
        for pid in [999, None]:
            self.terminal(pid)
            if pid is None:
                window = self.connection.create_resource_object('window', 9)
                window.get_full_property = lambda atom, kind: None
            tree = self.snapshot([Node('Office', 'application', [Node('Writer', 'frame')])])
            self.assertFalse(tree['complete'])


if __name__ == '__main__': unittest.main()
