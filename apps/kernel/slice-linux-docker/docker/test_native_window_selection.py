"""MP-08/MP-11: fallback and native workers share exact-PID bounded selection.
The physical native regression is capture-reparent.test.c; these are supplements.
"""
import ast
import unittest
from pathlib import Path


def selector(tree, owners, sizes, insets=None):
    source = ast.parse(Path(__file__).with_name('kernel-browser-xshm.py').read_text())
    function = next(node for node in source.body if isinstance(node, ast.FunctionDef) and node.name == 'owned_windows')
    context = {'children': lambda _display, parent: tree.get(parent, []),
               'pid_of': lambda _display, window: owners.get(window),
               'dims': lambda _display, window: sizes.get(window, (1, 1)),
               'frame_insets': lambda _display, window: (insets or {}).get(window, [0,0,0,0])}
    exec(compile(ast.Module(body=[function], type_ignores=[]), 'owned_windows', 'exec'), context)
    return lambda: context['owned_windows'](None, 0, 42, 128, 128)


class NativeWindowSelection(unittest.TestCase):
    def test_wm_frame_is_not_selected_and_owned_client_is(self):
        self.assertEqual(selector({0: [1], 1: [2]}, {1: 99, 2: 42}, {1: (128, 128), 2: (128, 140)})(), [2])

    def test_ambiguous_and_foreign_clients_remain_distinct(self):
        self.assertEqual(selector({0: [1, 2]}, {1: 42, 2: 42}, {1: (128, 128), 2: (128, 128)})(), [1, 2])
        self.assertEqual(selector({0: [1]}, {1: 99}, {1: (128, 128)})(), [])

    def test_partial_search_cannot_hide_another_owned_client(self):
        for tree in [{node: [node+1] for node in range(10)}, {0: list(range(1, 2050))}]:
            with self.assertRaisesRegex(ValueError, 'tree bound'):
                selector(tree, {}, {})()

    def test_client_frame_insets_preserve_the_exact_viewport(self):
        self.assertEqual(selector({0:[1]}, {1:42}, {1:(136,147)}, {1:[4,4,0,4]})(), [1])
        self.assertEqual(selector({0:[1]}, {1:42}, {1:(136,147)}, {1:[3,4,0,4]})(), [])
