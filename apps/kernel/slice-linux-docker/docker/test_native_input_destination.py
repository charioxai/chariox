"""MP-08 / MP-11: the page-vs-browser-UI hit test is placed from CDP and X11, never an accessibility tree."""
import importlib.util
import pathlib
import types
import unittest

def load(name, file):
    spec = importlib.util.spec_from_file_location(name, pathlib.Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

destination = load('native_input_destination', 'native-input-destination.py')
protection = load('browser_desktop_protection', 'browser-desktop-protection.py')

CLIENT = [40, 30, 900, 700]


def window(insets=None):
    return types.SimpleNamespace(get_geometry=lambda: types.SimpleNamespace(width=CLIENT[2], height=CLIENT[3]),
                                 insets=insets)


def connection():
    root = types.SimpleNamespace(translate_coords=lambda w, x, y: types.SimpleNamespace(x=CLIENT[0], y=CLIENT[1]))
    return types.SimpleNamespace(screen=lambda: types.SimpleNamespace(root=root))


ACCESSIBILITY = types.SimpleNamespace(_protection=protection, frame_insets=lambda c, w: [4, 4, 0, 4])
PAGE = {'url': 'https://example.test/a', 'window': CLIENT, 'native': [892, 553], 'overlay': False}


def hit(point, page=PAGE, accessibility=ACCESSIBILITY):
    return destination.browser_document_point(connection(), window(), point, page, accessibility)


class BrowserDocumentPoint(unittest.TestCase):
    def test_page_pixels_hit_and_browser_ui_does_not(self):
        self.assertTrue(hit((44, 173)))
        self.assertTrue(hit((935, 725)))
        self.assertFalse(hit((200, 100)))     # toolbar
        self.assertFalse(hit((44, 172)))      # tab strip row just above the page
        self.assertFalse(hit((936, 400)))     # right GTK inset
        self.assertFalse(hit((500, 726)))     # bottom GTK inset

    def test_non_web_pages_and_unproven_geometry_are_browser_ui(self):
        self.assertFalse(hit((500, 400), {**PAGE, 'url': 'chrome://newtab/'}))
        self.assertFalse(hit((500, 400), {**PAGE, 'url': 'chrome-untrusted://x/'}))
        self.assertFalse(hit((500, 400), None))
        self.assertFalse(hit((500, 400), {**PAGE, 'native': None}))
        self.assertFalse(hit((500, 400), {**PAGE, 'window': [41, 30, 900, 700]}))   # popup or moved window
        self.assertFalse(hit((500, 400), {**PAGE, 'native': [600, 553]}))           # side panel
        wrong = types.SimpleNamespace(_protection=protection, frame_insets=lambda c, w: [0, 0, 0, 0])
        self.assertFalse(hit((500, 400), accessibility=wrong))

    def test_omnibox_list_over_the_page_and_missing_proof_are_browser_ui(self):
        self.assertTrue(hit((500, 400)))
        self.assertFalse(hit((500, 400), {**PAGE, 'overlay': True}))       # list drawn in the same X client
        self.assertFalse(hit((500, 400), {k: v for k, v in PAGE.items() if k != 'overlay'}))
        self.assertFalse(hit((500, 400), {**PAGE, 'overlay': None}))

    def test_scale_two_hits_in_device_pixels(self):
        doubled = types.SimpleNamespace(_protection=protection, frame_insets=lambda c, w: [8, 8, 0, 8])
        page = {'url': 'https://example.test/', 'window': [20, 15, 450, 350], 'native': [442, 276], 'overlay': False}
        self.assertTrue(hit((48, 170), page, doubled))
        self.assertFalse(hit((48, 169), page, doubled))


if __name__ == '__main__':
    unittest.main()
