"""MP-08 / MP-11: another application's window survives bounded hidden menus."""
import importlib.util
import pathlib
import sys
import types
import unittest


class Node:
    def __init__(self, name, role, children=(), pid=200):
        self.name, self.role, self.children, self.pid = name, role, children, pid
        self.childCount = len(children)

    def getChildAtIndex(self, index): return self.children[index]
    def get_process_id(self): return self.pid
    def getRoleName(self): return self.role
    def getRole(self): return 0
    def getState(self): return types.SimpleNamespace(contains=lambda flag: False)
    def queryComponent(self): raise NotImplementedError
    def queryAction(self): raise NotImplementedError


class TraversalTest(unittest.TestCase):
    def test_mp08_other_window_before_deep_hidden_menu_budget(self):
        menus = [Node('Hidden menu', 'menu item') for _ in range(512)]
        editor = Node('Editor', 'application', [Node('Editor window', 'frame', [
            Node('Menu bar', 'menu bar', menus)])])
        canvas = Node('Canvas', 'application', [Node('Chariox pixel fixture', 'frame', pid=201)], pid=201)
        desktop = Node('Desktop', 'desktop', [editor, canvas])
        atspi = types.ModuleType('pyatspi')
        atspi.Registry = types.SimpleNamespace(getDesktop=lambda index: desktop)
        for name in ['ROLE_PASSWORD_TEXT', 'STATE_SHOWING', 'STATE_ENABLED', 'STATE_FOCUSED', 'STATE_EDITABLE', 'DESKTOP_COORDS']:
            setattr(atspi, name, 1)
        connection = types.SimpleNamespace(
            screen=lambda: types.SimpleNamespace(root=types.SimpleNamespace(get_full_property=lambda *args: None)),
            intern_atom=lambda value: value, close=lambda: None)
        xlib = types.ModuleType('Xlib')
        xlib.X = types.SimpleNamespace(AnyPropertyType=0)
        xlib.display = types.SimpleNamespace(Display=lambda: connection)
        saved = {name: sys.modules.get(name) for name in ['pyatspi', 'Xlib']}
        sys.modules.update(pyatspi=atspi, Xlib=xlib)
        try:
            spec = importlib.util.spec_from_file_location('native_accessibility', pathlib.Path(__file__).with_name('native-accessibility.py'))
            driver = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(driver)
            driver.alive = lambda process: True
            tree = driver.snapshot([{'pid': 200, 'started': '1'}, {'pid': 201, 'started': '2'}])
            self.assertTrue(tree['available'])
            self.assertFalse(tree['complete'])
            self.assertLessEqual(len(tree['nodes']), 512)
            node = next(node for node in tree['nodes'] if node['name'] == 'Chariox pixel fixture')
            self.assertEqual((node['pid'], node['started'], node['path']), (201, '2', [0]))
        finally:
            for name, module in saved.items():
                if module is None: sys.modules.pop(name, None)
                else: sys.modules[name] = module


if __name__ == '__main__': unittest.main()
