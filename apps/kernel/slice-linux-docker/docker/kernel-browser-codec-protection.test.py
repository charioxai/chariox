"""MP-11: actual codec input/output checks and reference recovery after drops."""
import base64
import importlib.util
import os
import unittest
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace
import av

spec=importlib.util.spec_from_file_location('stripes',Path(os.environ.get('MP_STRIPE_MODULE',Path(__file__).with_name('kernel-browser-stripes.py'))))
s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)
region=dict(x=20,y=20,width=40,height=40)

class ProtectionTests(unittest.TestCase):
    def test_offscreen_and_partial_intersections_in_input_and_decoded_guards(self):
        masks = [dict(x=-100,y=0,width=20,height=20),
                 dict(x=140,y=0,width=20,height=20),
                 dict(x=0,y=-100,width=20,height=20),
                 dict(x=0,y=140,width=20,height=20)]
        pixels = bytearray([255]) * (128*128*4)
        # Fully offscreen boxes have no intersection, including negative ends.
        s._protection.black_input(pixels,128,128,masks)
        rows=s.StripeEncoder().encode(pixels,128,128,8000000,True,'avc1.420033',masks)
        self.assertEqual(len(rows),8)
        for row in rows:
            guard=s._protection.DecodedMaskGuard(row['codec'])
            self.assertTrue(guard.safe(base64.b64decode(row['data_base64']),masks,128,row['height'],y=row['y']))
        partial = dict(x=-10,y=30,width=30,height=30)
        for y in range(30,60):
            for x in range(20):pixels[(y*128+x)*4:(y*128+x)*4+3]=bytes(3)
        s._protection.black_input(pixels,128,128,[partial])
        # Intersection is still enforced, both full-frame and per stripe.
        white=s.StripeEncoder().encode(bytes([255])*128*128*4,128,128,8000000,True)
        self.assertFalse(s._protection.DecodedMaskGuard('avc1.420033').safe(
            base64.b64decode(white[2]['data_base64']),[partial],128,white[2]['height'],y=white[2]['y']))
        with self.assertRaisesRegex(ValueError,'unmasked'):
            s.StripeEncoder().encode(bytes([255])*128*128*4,128,128,8000000,True,'avc1.420033',[partial])
        pixels[(40*128+10)*4]=255
        with self.assertRaisesRegex(ValueError,'unmasked'):
            s._protection.black_input(pixels,128,128,[partial])

    def test_unmasked_input_never_reaches_any_codec(self):
        e=s.StripeEncoder()
        def forbidden(*args):raise AssertionError('codec saw unmasked input')
        with patch.object(av,'CodecContext',SimpleNamespace(create=forbidden)):
            with self.assertRaisesRegex(ValueError,'unmasked'):
                e.encode(bytes([255])*128*128*4,128,128,8000000,True,'avc1.420033',[region])

    def test_uncertain_metadata_refuses_before_encode(self):
        for masks in [False, [dict(region,x=float('nan'))], [dict(region,width=-1)], [dict(x=0)]]:
            e=s.StripeEncoder()
            with self.assertRaises((ValueError,KeyError,TypeError)):
                e.encode(bytes(128*128*4),128,128,8000000,True,'avc1.420033',masks)

    def test_real_decode_detects_an_unprotected_reconstruction(self):
        rows=s.StripeEncoder().encode(bytes([255])*128*128*4,128,128,8000000,True)
        row=rows[1]
        guard=s._protection.DecodedMaskGuard(row['codec'])
        self.assertFalse(guard.safe(base64.b64decode(row['data_base64']),[region],128,row['height'],y=row['y']))

    def test_uncertain_row_drops_whole_batch_and_next_frame_is_independent(self):
        pixels=bytes(128*128*4);e=s.StripeEncoder()
        # Decoder failure is a security boundary: even other already-encoded
        # rows must not escape, and the following packet must reset its base.
        with patch.object(s._protection.DecodedMaskGuard,'safe',return_value=False):
            self.assertEqual(e.encode(pixels,128,128,8000000,True,'avc1.420033',[region]),[])
        recovered=e.encode(pixels,128,128,8000000,False,'avc1.420033',[region])
        self.assertEqual(len(recovered),8)
        self.assertTrue(all(row['key'] and row['reference_sequence'] is None for row in recovered))
        for row in recovered:
            d=av.CodecContext.create('h264','r')
            self.assertEqual(len(d.decode(av.Packet(base64.b64decode(row['data_base64'])))),1)

    def test_mask_change_cannot_reuse_an_older_codec_reference(self):
        pixels=bytes(128*128*4);e=s.StripeEncoder()
        e.encode(pixels,128,128,8000000,True,'avc1.420033',[region])
        rows=e.encode(pixels,128,128,8000000,False,'avc1.420033',[dict(region,x=50)])
        self.assertEqual(len(rows),8)
        self.assertTrue(all(row['key'] for row in rows))

if __name__=='__main__':unittest.main()
