"""MP-08 / MP-11: desktop placement of CDP protection regions fails closed.

The page viewport is placed from CDP window bounds, the X11 client geometry
with its GTK insets and the native Tab size; no accessibility tree is read.
"""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('browser_desktop_protection', pathlib.Path(__file__).with_name('browser-desktop-protection.py'))
protection = importlib.util.module_from_spec(spec)
spec.loader.exec_module(protection)

# A 900x700 window at (40, 30): 143 px of Tab strip and toolbar above a 892x553 page, 4 px GTK sides and bottom.
CLIENT, INSETS = [40, 30, 900, 700], [4, 4, 0, 4]


def page(**changes):
    value = {'url': 'https://example.test/a', 'window': CLIENT, 'native': [892, 553], 'viewport': [892, 553], 'dpr': 1, 'zoom': 1,
             'regions': [[100, 50, 84, 32]], 'chrome': False}
    value.update(changes)
    return value


class ViewportRect(unittest.TestCase):
    def test_native_tab_fills_the_client_width_and_sits_on_its_bottom_inset(self):
        self.assertEqual(protection.viewport_rect(page(), CLIENT, INSETS), [44, 173, 892, 553])

    def test_device_scale_two_derives_from_the_cdp_window(self):
        scaled = page(window=[0, 0, 800, 500], native=[792, 410], viewport=[1584, 820], dpr=2)
        self.assertEqual(protection.viewport_rect(scaled, [0, 0, 1600, 1000], [8, 8, 0, 8]), [8, 172, 1584, 820])

    def test_every_disagreement_between_sources_refuses(self):
        for name, value, client, insets in [
            ('side panel narrows the Tab', page(native=[600, 553]), CLIENT, INSETS),
            ('extension unavailable', page(native=None), CLIENT, INSETS),
            ('non-integer native size', page(native=[892.0, 553]), CLIENT, INSETS),
            ('window moved', page(window=[41, 30, 900, 700]), CLIENT, INSETS),
            ('window resized', page(window=[40, 30, 900, 650]), CLIENT, INSETS),
            ('Tab taller than the client', page(native=[892, 900]), CLIENT, INSETS),
            ('implausibly tall browser UI', page(native=[892, 100]), CLIENT, INSETS),
            ('short insets', page(), CLIENT, [4, 4, 0]),
        ]:
            with self.subTest(name):
                self.assertIsNone(protection.viewport_rect(value, client, insets))


class WindowMasks(unittest.TestCase):
    def test_places_regions_at_the_derived_origin_with_padding(self):
        self.assertEqual(protection.window_masks({'pages': [page()]}, CLIENT, INSETS), [[140, 219, 92, 40]])

    def test_scale_two_pads_in_device_pixels_and_clips_to_the_viewport(self):
        scaled = page(window=[0, 0, 800, 500], native=[792, 410], viewport=[1584, 820], dpr=2, regions=[[0, 0, 10, 10]])
        self.assertEqual(protection.window_masks({'pages': [scaled]}, [0, 0, 1600, 1000], [8, 8, 0, 8]), [[8, 172, 18, 18]])

    def test_page_zoom_binds_and_moves_no_origin_but_emulated_density_is_withheld(self):
        self.assertEqual(protection.window_masks({'pages': [page(dpr=1.25, zoom=1.25)]}, CLIENT, INSETS), [[140, 219, 92, 40]])
        emulated = page(window=[0, 0, 1280, 800], native=[1280, 657], viewport=[1280, 657], dpr=2, zoom=1)
        self.assertIsNone(protection.window_masks({'pages': [emulated]}, [0, 0, 1280, 800], [0, 0, 0, 0]))

    def test_the_emulated_viewport_may_exceed_the_native_tab_but_never_undercut_it(self):
        emulated = page(viewport=[1280, 800], regions=[[100, 50, 84, 32]])
        self.assertEqual(protection.window_masks({'pages': [emulated]}, CLIENT, INSETS), [[140, 219, 92, 40]])
        edge = page(viewport=[1280, 800], regions=[[880, 540, 100, 100]])   # runs past the Tab view: clipped to it
        self.assertEqual(protection.window_masks({'pages': [edge]}, CLIENT, INSETS), [[920, 709, 16, 17]])
        self.assertIsNone(protection.window_masks({'pages': [page(viewport=[800, 500])]}, CLIENT, INSETS))

    def test_vault_policy_leaves_browser_chrome_and_status_visible(self):
        self.assertEqual(protection.window_masks({'pages': [page(chrome=True, regions=[])]}, CLIENT, INSETS), [])

    def test_withholds_without_a_one_to_one_binding(self):
        for value in [
            None, {'pages': []},
            {'pages': [page(viewport=[600, 553])]},       # page smaller than its Tab: docked devtools or emulated size
            {'pages': [page(viewport=[892, 400])]},
            {'pages': [page(window=[41, 30, 900, 700])]}, # window moved
            {'pages': [page(), page()]},                  # two windows of one geometry
            {'pages': [page(regions=[[1.5, 2, 3, 4]])]},  # unrounded region
            {'pages': [page(window=[40, 30, 900, 650])]}, # same width, different height
            {'pages': [page(zoom=None)]},
            {'pages': [page(native=None)]},               # extension unavailable
        ]:
            with self.subTest(value=value):
                self.assertIsNone(protection.window_masks(value, CLIENT, INSETS))

    def test_wrong_insets_are_caught_by_the_width_proof(self):
        self.assertIsNone(protection.window_masks({'pages': [page()]}, CLIENT, [0, 0, 0, 0]))
        self.assertIsNone(protection.window_masks({'pages': [page()]}, CLIENT, [30, 4, 0, 4]))


if __name__ == '__main__':
    unittest.main()
