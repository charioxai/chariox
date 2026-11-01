"""MP-11: a Vault-filled kernel-browser window is covered only while its field cannot be placed."""
import importlib.util
import pathlib
import sys
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native_accessibility', pathlib.Path(__file__).with_name('native-accessibility.py'))
driver = importlib.util.module_from_spec(spec)
# The fill logic never reads the AT-SPI tree, and CI runners do not ship pyatspi.
_installed_pyatspi = sys.modules.get('pyatspi')
sys.modules['pyatspi'] = types.ModuleType('pyatspi')
try:
    spec.loader.exec_module(driver)
finally:
    if _installed_pyatspi is None:
        sys.modules.pop('pyatspi', None)
    else:
        sys.modules['pyatspi'] = _installed_pyatspi
FRAME = [0, 0, 400, 300]


def fake_display(*args):
    window = types.SimpleNamespace(
        get_attributes=lambda: types.SimpleNamespace(map_state=2),
        get_full_property=lambda atom, kind: types.SimpleNamespace(value=[4242]),
        get_geometry=lambda: types.SimpleNamespace(width=400, height=300))
    root = types.SimpleNamespace(
        get_full_property=lambda atom, kind: types.SimpleNamespace(value=[10]),
        translate_coords=lambda window, x, y: types.SimpleNamespace(x=0, y=0))
    return types.SimpleNamespace(
        screen=lambda: types.SimpleNamespace(root=root, width_in_pixels=1280, height_in_pixels=800),
        intern_atom=lambda name: name, create_resource_object=lambda kind, id: window, close=lambda: None)


class FillCoverageTest(unittest.TestCase):
    def capture(self, protection, fill, placed=None):
        with patch.object(driver._x11_module, 'open_display', fake_display), \
                patch.object(driver, 'alive', lambda process: True), \
                patch.object(driver, 'frame_rect', lambda root, window: FRAME), \
                patch.object(driver, 'frame_insets', lambda connection, window: (0, 0, 0, 0)), \
                patch.object(driver._fills, 'regions', return_value=[]), \
                patch.object(driver._protection, '_screen_scale', lambda page_window, client: 1), \
                patch.object(driver._protection, 'window_masks', lambda *args: placed):
            return driver.capture_snapshot([], [{'pid': 4242, 'started': '1'}], protection, [], fill)

    def test_an_adopted_measurement_without_a_visible_field_masks_nothing(self):
        # Password dots: the filled field has no plain region, so there is nothing to cover.
        result = self.capture({'pages': [{'regions': []}]}, True)
        self.assertEqual((result['masks'], result['unplaced']), ([], False))

    def test_before_any_measurement_is_adopted_the_window_is_covered_whole(self):
        result = self.capture(None, True)
        self.assertEqual((result['masks'], result['unplaced']), ([FRAME], True))

    def test_a_visible_field_that_cannot_be_placed_covers_its_window_instead_of_refusing(self):
        page = {'regions': [[1, 2, 3, 4]], 'window': [0, 0, 400, 300]}
        result = self.capture({'pages': [page]}, True)
        self.assertEqual((result['masks'], result['unplaced']), ([FRAME], True))
        with self.assertRaises(ValueError):
            self.capture({'pages': [page]}, None)

    def test_a_placed_field_masks_only_its_region(self):
        page = {'regions': [[1, 2, 3, 4]], 'window': [0, 0, 400, 300]}
        result = self.capture({'pages': [page]}, True, placed=[[10, 20, 30, 40]])
        self.assertEqual((result['masks'], result['unplaced']), ([[10, 20, 30, 40]], False))


if __name__ == '__main__':
    unittest.main()
