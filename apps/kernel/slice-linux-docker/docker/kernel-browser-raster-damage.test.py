"""MD-DISPLAY-02/04: drain/paint/readback race, reconstructed fingerprint."""
import hashlib
import importlib.util
from pathlib import Path
import unittest
import sys
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('raster_damage', Path(__file__).with_name('kernel-browser-raster-damage.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def reconstruct(previous, raw, width, height, drained):
    patch, payload = module.raster_payload(previous, raw, width, height, drained)
    if patch is None:
        return payload
    restored = bytearray(previous)
    left, top, right, bottom = patch
    for row in range(top, bottom):
        offset = (row-top)*(right-left)*4
        restored[(row*width+left)*4:(row*width+right)*4] = payload[offset:offset+(right-left)*4]
    return bytes(restored)


class DamageTest(unittest.TestCase):
    def test_mp10_native_geometry_is_bounded_to_supported_pairs(self):
        for pair in [(1280,800),(2560,1600),(1920,1080)]:
            self.assertTrue(module.capture_geometry_allowed(*pair))
        for pair in [(1920,800),(1280,1080),(3840,2160),(1920.0,1080),(True,1080)]:
            self.assertFalse(module.capture_geometry_allowed(*pair))

    def test_disjoint_paint_after_damage_drain_is_in_the_readback_packet(self):
        width, height = 128, 80
        previous = bytes(width*height*4)
        framebuffer = bytearray(previous)
        def paint(x, y, colour):
            framebuffer[(y*width+x)*4:(y*width+x+1)*4] = bytes(colour)
        paint(4, 4, [11, 22, 33, 0])
        drained = [4, 4, 5, 5]  # R1 event already consumed.
        paint(100, 6, [44, 55, 66, 0])  # R2 paints before readback is serviced.
        raw = bytes(framebuffer)
        restored = reconstruct(previous, raw, width, height, drained)
        self.assertEqual(hashlib.sha256(restored).digest(), hashlib.sha256(raw).digest())
        self.assertEqual(restored, raw)
        # R2's delayed event must not be needed to repair a packet claiming raw's signature.
        self.assertEqual(reconstruct(restored, raw, width, height, [100, 6, 101, 7]), raw)

    def test_all_readback_edges_and_disjoint_rows_reconstruct(self):
        width,height=128,80
        previous=bytes(width*height*4)
        for points in [[(0,0)],[(127,79)],[(127,0),(0,79)],[(2,40),(100,42)],[(0,79),(127,79)]]:
            raw=bytearray(previous)
            for x,y in points:raw[(y*width+x)*4]=255
            self.assertEqual(reconstruct(previous,bytes(raw),width,height,[0,0,1,1]),raw)

    def test_missing_events_and_large_damage_remain_complete(self):
        width, height = 128, 80
        previous = bytes(width*height*4)
        for raw in [bytes([9])*len(previous), previous[:-4]+bytes([1, 2, 3, 0])]:
            self.assertEqual(reconstruct(previous, raw, width, height, [0, 0, 1, 1]), raw)
        self.assertEqual(reconstruct(None, previous, width, height, [0, 0, width, height]), previous)


class FingerprintTest(unittest.TestCase):
    def test_sparse_bounds_reuse_only_the_same_complete_byte_comparison(self):
        width,height=128,160
        before=bytes(width*height*4);after=bytearray(before)
        after[(70*width+105)*4]=12;after[(72*width+2)*4]=23;after=bytes(after)
        fingerprint=module.RasterFingerprint(width,height)
        fingerprint.update(before)
        fingerprint.update(bytes(bytearray(before)))  # delayed unchanged event
        fingerprint.update(after)
        patch,payload=module.raster_payload(before,after,width,height,[100,70,110,71],fingerprint)
        self.assertEqual(patch,[0,70,112,73])
        restored=bytearray(before)
        left,top,right,bottom=patch
        for row in range(top,bottom):
            start=(row-top)*(right-left)*4
            restored[(row*width+left)*4:(row*width+right)*4]=payload[start:start+(right-left)*4]
        self.assertEqual(restored,after)
        # A cache from another readback cannot restrict sparse search bounds.
        newer=bytearray(after);newer[-4]=77;newer=bytes(newer)
        self.assertEqual(module.raster_payload(after,newer,width,height,[],fingerprint),
                         module.raster_payload(after,newer,width,height,[]))
        different_before=bytes([1])*len(before)
        self.assertEqual(module.raster_payload(different_before,after,width,height,[],fingerprint),
                         module.raster_payload(different_before,after,width,height,[]))

    def test_cached_fingerprint_matches_independent_reconstructed_raster(self):
        width,height=128,160
        raw=bytes(width*height*4)
        cache=module.RasterFingerprint(width,height)
        initial=cache.update(raw)
        self.assertEqual(initial,module.RasterFingerprint(width,height).update(raw))
        newer=bytearray(raw);newer[4:8]=bytes([1,2,3,0]);newer[-4:]=bytes([4,5,6,0]);newer=bytes(newer)
        restored=reconstruct(raw,newer,width,height,[1,0,2,1])
        result=cache.update(newer)
        self.assertNotEqual(result,initial)
        self.assertEqual(result,module.RasterFingerprint(width,height).update(restored))
        self.assertEqual(cache.update(newer),result)
        self.assertEqual(cache.update(raw),initial)
        self.assertNotEqual(initial,module.RasterFingerprint(256,80).update(raw),'geometry binds band interpretation')
        with self.assertRaises(ValueError):cache.update(raw[:-4])

class CollisionTest(unittest.TestCase):
    def test_mp11_hash_collision_still_exposes_changed_bands(self):
        original=module.fast_hash
        module.fast_hash=lambda data:0
        try:
            cache=module.RasterFingerprint(128,80)
            first=cache.update(bytes(128*80*4))
            second=cache.update(bytes([1])*(128*80*4))
            self.assertEqual(first,second)
            self.assertTrue(cache.changed_bands, 'hash collision cannot suppress publication')
        finally:module.fast_hash=original

class InputWakeTest(unittest.TestCase):
    def test_mp10_wake_is_damage_only_bounded_and_not_a_poll_loop(self):
        self.assertFalse(module.capture_due(10,False,10.001,10.1))
        self.assertFalse(module.capture_due(10,True,10.001,0))
        self.assertTrue(module.capture_due(10,True,10.001,10.1))
        self.assertFalse(module.capture_due(10,True,10.001,10.0005))
        self.assertTrue(module.capture_due(10,True,10.016,0))
        self.assertEqual(module.capture_wait(10,False,10.001,10.1),1)
        self.assertEqual(module.capture_wait(10,True,10.001,10.1),0)
        self.assertAlmostEqual(module.capture_wait(10,True,10.001,0),.015)

if __name__ == '__main__':
    unittest.main()
