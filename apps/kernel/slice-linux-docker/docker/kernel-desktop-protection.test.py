"""MP-08/MP-11: desktop protection oracle answers masks only for complete, unprotected snapshots."""
import importlib.util
import unittest
from pathlib import Path
spec = importlib.util.spec_from_file_location('kernel_desktop_protection', Path(__file__).with_name('kernel-desktop-protection.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
REQUEST = {'processes': [], 'browser_processes': [], 'browser_protection': None}


class OracleTests(unittest.TestCase):
    def test_complete_unprotected_snapshot_returns_its_masks_and_a_stable_digest(self):
        tree = {'available': True, 'complete': True, 'protected': False, 'masks': [[1, 2, 3, 4]], 'nodes': []}
        first = module.answer(REQUEST, lambda *args: dict(tree))
        self.assertEqual(first['masks'], [[1, 2, 3, 4]])
        self.assertEqual(first['digest'], module.answer(REQUEST, lambda *args: dict(tree))['digest'])

    def test_incomplete_unavailable_or_protected_snapshots_mask_everything(self):
        for change in ({'complete': False}, {'available': False}, {'protected': True}):
            tree = {'available': True, 'complete': True, 'protected': False, 'masks': [], **change}
            self.assertIsNone(module.answer(REQUEST, lambda *args: tree)['masks'], change)

    def test_any_snapshot_change_changes_the_digest(self):
        tree = {'available': True, 'complete': True, 'protected': False, 'masks': [], 'nodes': [{'name': 'a'}]}
        other = {**tree, 'nodes': [{'name': 'b'}]}
        self.assertNotEqual(module.answer(REQUEST, lambda *args: tree)['digest'], module.answer(REQUEST, lambda *args: other)['digest'])

    def test_request_scope_is_exact_and_bounded(self):
        for bad in ({**REQUEST, 'extra': 1}, {**REQUEST, 'processes': [{}] * 257}, {**REQUEST, 'browser_protection': []}):
            with self.assertRaises(ValueError):
                module.scope(bad)


if __name__ == '__main__':
    unittest.main()
