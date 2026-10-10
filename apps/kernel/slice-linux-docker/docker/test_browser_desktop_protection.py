"""MP-08 / MP-11: desktop placement of CDP protection regions fails closed."""
import importlib.util
import pathlib
import types
import unittest

spec = importlib.util.spec_from_file_location('browser_desktop_protection', pathlib.Path(__file__).with_name('browser-desktop-protection.py'))
protection = importlib.util.module_from_spec(spec)
spec.loader.exec_module(protection)

CLIENT, FRAME = [40, 30, 900, 700], [40, 30, 900, 700]
DOC = {'uri': 'https://example.test/a#top', 'rect': [44, 173, 892, 553]}


def page(**changes):
    value = {'url': 'https://example.test/a', 'window': CLIENT, 'viewport': [892, 553], 'dpr': 1, 'zoom': 1,
             'regions': [[100, 50, 84, 32]], 'chrome': False}
    value.update(changes)
    return value


class WindowMasks(unittest.TestCase):
    def test_places_regions_at_the_proven_document_origin_with_padding(self):
        self.assertEqual(protection.window_masks({'pages': [page()]}, CLIENT, FRAME, [DOC]), [[140, 219, 92, 40]])

    def test_scale_two_pads_in_device_pixels_and_clips_to_the_document(self):
        doc = {'uri': DOC['uri'], 'rect': [8, 286, 1584, 706]}
        masks = protection.window_masks({'pages': [page(window=[0, 0, 800, 500], viewport=[1584, 706], dpr=2, regions=[[0, 0, 10, 10]])]}, [0, 0, 1600, 1000], [0, 0, 1600, 1000], [doc])
        self.assertEqual(masks, [[8, 286, 18, 18]])

    def test_page_zoom_binds_but_emulated_density_is_withheld(self):
        self.assertEqual(protection.window_masks({'pages': [page(dpr=1.25, zoom=1.25)]}, CLIENT, FRAME, [DOC]), [[140, 219, 92, 40]])
        # Host display mode emulates DSF 2 (and view scale) in a DSF 1 window: no provable mapping.
        emulated = page(window=[0, 0, 1280, 800], viewport=[1280, 800], dpr=2, zoom=1)
        self.assertIsNone(protection.window_masks({'pages': [emulated]}, [0, 0, 1280, 800], [0, 0, 1280, 800], [{'uri': DOC['uri'], 'rect': [0, 0, 1280, 800]}]))

    def test_mp11_vault_policy_leaves_browser_chrome_and_status_visible(self):
        masks = protection.window_masks({'pages': [page(chrome=True, regions=[])]}, CLIENT, [40, 10, 900, 720], [DOC])
        self.assertEqual(masks, [])

    def test_withholds_without_a_one_to_one_binding(self):
        cases = [
            None, {'pages': []},
            {'pages': [page(url='https://other.test/')]},                  # navigated: wrong document
            {'pages': [page(viewport=[600, 553])]},                         # devtools/side panel split
            {'pages': [page(window=[41, 30, 900, 700])]},                   # window moved/resized
            {'pages': [page(), page()]},                                    # ambiguous pages
            {'pages': [page(regions=[[1.5, 2, 3, 4]])]},                    # unrounded region
            {'pages': [page(window=[40, 30, 900, 650])]},                   # same width, different height
            {'pages': [page(zoom=None)]},
        ]
        for value in cases:
            with self.subTest(value=value):
                self.assertIsNone(protection.window_masks(value, CLIENT, FRAME, [DOC]))
        devtools = {'uri': 'devtools://devtools/bundled/devtools_app.html', 'rect': [400, 173, 536, 553]}
        self.assertIsNone(protection.window_masks({'pages': [page()]}, CLIENT, FRAME, [DOC, devtools]))
        self.assertIsNone(protection.window_masks({'pages': [page()]}, CLIENT, FRAME, []))


class Node:
    def __init__(self, role, children=(), rect=(0, 0, 0, 0), uri=''):
        self.role, self.children, self.childCount, self.uri = role, list(children), len(children), uri
        self.rect = types.SimpleNamespace(x=rect[0], y=rect[1], width=rect[2], height=rect[3])
        self.parent = None
        for child in self.children:
            child.parent = self
    def queryCollection(self): raise NotImplementedError
    def getRoleName(self): return self.role
    def getChildAtIndex(self, index): return self.children[index]
    def queryComponent(self): return types.SimpleNamespace(getExtents=lambda coords: self.rect)
    def queryDocument(self): return types.SimpleNamespace(getAttributeValue=lambda key: self.uri if key == 'URI' else '')


class Collected(Node):
    # Chromium's in-process collection: every document web, including frame documents.
    def queryCollection(self):
        def walk(node):
            return ([node] if node.role == 'document web' else []) + [match for child in node.children for match in walk(child)]
        return types.SimpleNamespace(MATCH_NONE=0, MATCH_ANY=1, SORT_ORDER_CANONICAL=0,
            createMatchRule=lambda *args: None, getMatches=lambda rule, order, count, traverse: walk(self)[:count])


class DocumentRects(unittest.TestCase):
    atspi = types.SimpleNamespace(DESKTOP_COORDS=0, ROLE_DOCUMENT_WEB=95, StateSet=lambda: None)

    def test_collection_returns_outermost_documents_only(self):
        frame_document = Node('document web', [], (60, 200, 100, 50), 'https://frame.test/')
        web = Node('document web', [Node('panel', [Node('internal frame', [frame_document])])], (44, 173, 892, 553), 'https://example.test/a')
        app = Collected('application', [Node('frame', [Node('panel', [web, Node('document web', rect=(159, 71, 0, 0))])])])
        self.assertEqual(protection.document_rects(app, self.atspi), [{'uri': 'https://example.test/a', 'rect': [44, 173, 892, 553]}])
        broken = Collected('application', [Node('frame', [web])])
        broken.queryCollection = lambda: (_ for _ in ()).throw(TypeError('incompatible collection binding'))
        self.assertEqual(protection.document_rects(broken, self.atspi), [{'uri': 'https://example.test/a', 'rect': [44, 173, 892, 553]}])
        many = Collected('application', [Node('document web', rect=(0, 0, 1, 1)) for _ in range(protection.MAX_DOCUMENTS+1)])
        with self.assertRaises(ValueError):
            protection.document_rects(many, self.atspi)

    def test_collects_sized_documents_without_entering_web_content(self):
        web = Node('document web', [Node('entry')], (44, 173, 892, 553), 'https://example.test/a')
        app = Node('application', [Node('frame', [Node('panel', [web, Node('document web', rect=(159, 71, 0, 0))])])])
        self.assertEqual(protection.document_rects(app, self.atspi), [{'uri': 'https://example.test/a', 'rect': [44, 173, 892, 553]}])

    def test_unbounded_trees_fail_closed(self):
        deep = Node('panel')
        for _ in range(protection.MAX_DEPTH+1):
            deep = Node('panel', [deep])
        with self.assertRaises(ValueError):
            protection.document_rects(Node('application', [Node('panel', [Node('panel')])]*protection.MAX_NODES), self.atspi)
        with self.assertRaises(ValueError):
            protection.document_rects(deep, self.atspi)


if __name__ == '__main__':
    unittest.main()
