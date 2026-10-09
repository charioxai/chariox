"""MP-08/MP-10/MP-11: autonomous masking before pixels/OCR reach an agent."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
from PIL import Image

spec = importlib.util.spec_from_file_location('observation_mask', Path(__file__).with_name('slice-observation-mask.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class MaskTests(unittest.TestCase):
    def test_mp08_mp11_registration_without_a_fill_target_masks_nothing(self):
        import os
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, XDG_RUNTIME_DIR=root):
            self.assertEqual(module.locate_regions({'targets': [], 'values': ['public-test-canary']}), [])

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
