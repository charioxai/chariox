"""MP-08/MP-10/MP-11: autonomous masking before pixels/OCR reach an agent."""
import io
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
import sys
import types
from PIL import Image

spec = importlib.util.spec_from_file_location('observation_mask', Path(__file__).with_name('slice-observation-mask.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class MaskTests(unittest.TestCase):
    def setUp(self):
        self.native = patch.object(module, 'native_coverage', create=True, return_value={
            'available': True, 'complete': True, 'protected': False, 'masks': []})
        self.native.start()
        self.addCleanup(self.native.stop)

    def test_mp11_empty_registry_room_screenshot_and_ocr_apply_native_masks(self):
        coverage = {'available': True, 'complete': True, 'protected': False,
                    'nodes': [], 'masks': [[20, 10, 20, 10]], 'uncovered': [[20, 10, 20, 10]]}
        policy = {'unknown': False, 'targets': [], 'values': []}
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'room.png'
            native = Mock(return_value=coverage)
            capture = lambda: Image.new('RGB', (80, 40), 'white')
            with patch.object(module, 'native_coverage', native):
                module.observe('screenshot', str(path), policy, lambda _: [], capture)
            with self.subTest(mode='screenshot'), Image.open(path) as image:
                self.assertEqual(image.getpixel((25, 15)), (0, 0, 0))
                self.assertEqual(image.getpixel((0, 0)), (255, 255, 255))
                self.assertEqual(image.getpixel((40, 15)), (255, 255, 255))
                self.assertEqual(image.getpixel((45, 15)), (255, 255, 255))
            def recognize(args, **kwargs):
                with Image.open(args[3]) as image:
                    # A recognizer cannot see the canary once its pixels are masked.
                    print('public' if image.getpixel((25, 15)) == (0, 0, 0) else 'private-canary')
            with patch('sys.stdout', new_callable=io.StringIO) as text:
                with patch.object(module, 'native_coverage', native):
                    module.observe('ocr', None, policy, lambda _: [], capture, recognize, root)
            with self.subTest(mode='ocr'):
                self.assertEqual(text.getvalue(), 'public\n')
                self.assertEqual(native.call_count, 4)

    def test_mp11_room_native_coverage_change_retries_before_releasing_pixels(self):
        public = {'available': True, 'complete': True, 'protected': False, 'masks': []}
        masked = {**public, 'masks': [[20, 10, 20, 10]]}
        native = Mock(side_effect=[public, masked, masked, masked])
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'room.png'
            capture = Mock(side_effect=lambda: Image.new('RGB', (80, 40), 'white'))
            with patch.object(module, 'native_coverage', native):
                module.observe('screenshot', str(path), {}, lambda _: [], capture)
            self.assertEqual(capture.call_count, 2)
            with Image.open(path) as image:
                self.assertEqual(image.getpixel((25, 15)), (0, 0, 0))

    def test_destroyed_native_window_is_pruned_but_other_x_errors_fail_closed(self):
        class BadWindow(Exception):
            pass
        window = Mock()
        window.get_attributes.side_effect = BadWindow()
        connection = Mock()
        connection.create_resource_object.return_value = window
        modules = {
            'selkies': types.ModuleType('selkies'),
            'selkies.Xlib': types.SimpleNamespace(display=types.SimpleNamespace(Display=lambda: connection), error=types.SimpleNamespace(BadWindow=BadWindow)),
        }
        target = {'kind': 'native', 'target': {'focus_window': 42, 'active_window': 42}}
        policy = {'targets': [target]}
        modules['Xlib'] = modules['selkies.Xlib']
        with patch.dict(sys.modules, modules):
            self.assertEqual(module.locate_regions(policy), [])
            self.assertEqual(policy['targets'], [])
            self.assertTrue(connection.close.called)
            window.get_attributes.side_effect = RuntimeError('transport failed')
            with self.assertRaises(RuntimeError):
                module.locate_regions({'targets': [target]})

    def test_dead_native_targets_are_reported_to_the_kernel_after_capture(self):
        target = {'kind': 'native', 'target': {'focus_window': 42, 'active_window': 42}}
        policy = {'targets': [target]}
        def locate(policy):
            policy['targets'] = []
            return []
        with tempfile.TemporaryDirectory() as root, patch('sys.stderr', new_callable=io.StringIO) as receipt:
            module.observe('screenshot', str(Path(root) / 'masked.png'), policy, locate,
                           lambda: Image.new('RGB', (40, 40), 'white'))
            self.assertEqual(receipt.getvalue(), 'CHARIOX_OBSERVATION_PRUNED_NATIVE:[42]\n')

    def test_dead_native_target_receipt_survives_a_failed_capture(self):
        target = {'kind': 'native', 'target': {'focus_window': 42, 'active_window': 42}}
        policy = {'targets': [target]}
        def locate(policy):
            policy['targets'] = []
            return []
        with patch('sys.stderr', new_callable=io.StringIO) as receipt:
            with self.assertRaises(module.ObservationRedacted):
                module.observe('screenshot', None, policy, locate, Mock(side_effect=RuntimeError('capture failed')))
            self.assertEqual(receipt.getvalue(), 'CHARIOX_OBSERVATION_PRUNED_NATIVE:[42]\n')

    def test_only_inserted_region_is_masked_and_benign_pixels_survive(self):
        image = Image.new('RGB', (80, 40), 'white')
        masked = module.mask_image(image, [[20, 10, 20, 10]])
        self.assertEqual(masked.getpixel((25, 15)), (0, 0, 0))
        self.assertEqual(masked.getpixel((0, 0)), (255, 255, 255))
        self.assertEqual(image.getpixel((25, 15)), (255, 255, 255))

    def test_stale_region_is_dropped_then_relocated_and_recaptured_automatically(self):
        locate = Mock(side_effect=[[[1, 1, 10, 10]], [[2, 1, 10, 10]], [[2, 1, 10, 10]], [[2, 1, 10, 10]]])
        capture = Mock(side_effect=lambda: Image.new('RGB', (40, 40), 'white'))
        image = module.capture_masked({}, locate, capture)
        self.assertEqual(capture.call_count, 2)
        self.assertEqual(image.getpixel((5, 5)), (0, 0, 0))

    def test_unknown_region_retries_are_bounded_with_no_human_callback(self):
        locate = Mock(side_effect=ValueError('unknown'))
        capture = Mock()
        with self.assertRaisesRegex(module.ObservationRedacted, 'observation redacted, retrying'):
            module.capture_masked({}, locate, capture)
        self.assertEqual(locate.call_count, 3)
        capture.assert_not_called()

    def test_invalid_region_cannot_release_raw_pixels(self):
        for regions in ([[0, 0, 0, 1]], [[float('nan'), 1, 2, 3]]):
            with self.assertRaises(module.ObservationRedacted):
                module.capture_masked({}, lambda _: regions, lambda: Image.new('RGB', (40, 40), 'white'))

    def test_known_offscreen_region_does_not_block_benign_pixels(self):
        image = module.capture_masked({}, lambda _: [[100, 100, 10, 10]], lambda: Image.new('RGB', (40, 40), 'white'))
        self.assertEqual(image.getpixel((5, 5)), (255, 255, 255))

    def test_clean_room_needs_no_masks_and_has_no_false_positive_block(self):
        image = module.capture_masked({}, lambda _: [], lambda: Image.new('RGB', (40, 40), 'white'))
        self.assertEqual(image.getpixel((5, 5)), (255, 255, 255))

    def test_ocr_receives_only_masked_pixels_and_uncached_fresh_capture(self):
        with tempfile.TemporaryDirectory() as root:
            old = Path(root) / 'old.png'
            Image.new('RGB', (40, 40), 'red').save(old)
            ocr = Mock(side_effect=lambda args, **kw: self.assertEqual(Image.open(args[3]).getpixel((10, 10)), (0, 0, 0)))
            module.observe('ocr', str(old), {}, lambda _: [[5, 5, 10, 10]], lambda: Image.new('RGB', (40, 40), 'white'), ocr, root)
            self.assertEqual(ocr.call_count, 1)
            self.assertEqual(list(Path(root).iterdir()), [old])
            self.assertEqual(Image.open(old).getpixel((10, 10)), (255, 0, 0))

if __name__ == '__main__':
    unittest.main()
