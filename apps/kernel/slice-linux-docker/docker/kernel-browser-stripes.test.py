"""MP-08/MP-10/MP-11: real codec row references, idle and collision checks."""
import base64,importlib.util,unittest,sys
from pathlib import Path
import av
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('stripes',Path(__file__).with_name('kernel-browser-stripes.py'))
s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)
class StripeTests(unittest.TestCase):
 def test_real_row_chains_loss_and_idle(self):
  e=s.StripeEncoder();pixels=bytearray(128*128*4);decoders={}
  first=e.encode(pixels,128,128,8000000)
  self.assertEqual(len(first),8);self.assertTrue(all(r['key'] and r['reference_sequence'] is None for r in first))
  for row in first:
   d=av.CodecContext.create('h264','r');decoders[row['row']]=d
   frame=d.decode(av.Packet(base64.b64decode(row['data_base64'])))[0]
   self.assertEqual((frame.width,frame.height),(128,row['height']))
  self.assertEqual(e.encode(pixels,128,128,8000000),[])
  pixels[4]=100
  second=e.encode(pixels,128,128,8000000)
  self.assertEqual([r['row'] for r in second],[0]);self.assertEqual(second[0]['reference_sequence'],1)
  decoders[0].decode(av.Packet(base64.b64decode(second[0]['data_base64'])))
  # Lose second row0: reset only it. Row1 chain remains at its delivered base.
  recovered=e.encode(pixels,128,128,8000000,[0])
  self.assertEqual([r['row'] for r in recovered],[0]);self.assertTrue(recovered[0]['key'])
  av.CodecContext.create('h264','r').decode(av.Packet(base64.b64decode(recovered[0]['data_base64'])))
  pixels[16*128*4+4]=77
  row1=e.encode(pixels,128,128,8000000)
  self.assertEqual([r['row'] for r in row1],[1]);self.assertEqual(row1[0]['reference_sequence'],1)
  decoders[1].decode(av.Packet(base64.b64decode(row1[0]['data_base64'])))
  self.assertEqual(len(e.encode(pixels,128,128,8000000,True)),8)
 def test_collision_compares_exact_bytes(self):
  original=s._xxh;s._xxh=lambda *args:0
  try:
   e=s.StripeEncoder();pixels=bytearray(128*128*4);e.encode(pixels,128,128,8000000)
   pixels[0]=255;self.assertEqual([r['row'] for r in e.encode(pixels,128,128,8000000)],[0])
  finally:s._xxh=original
 def test_vp8_real_decode_and_delta(self):
  e=s.StripeEncoder();pixels=bytearray(128*128*4)
  first=e.encode(pixels,128,128,8000000,True,"vp8");self.assertEqual(len(first),8)
  d=av.CodecContext.create('vp8','r');d.decode(av.Packet(base64.b64decode(first[0]['data_base64'])))
  pixels[0]=255;second=e.encode(pixels,128,128,8000000,False,"vp8")
  self.assertEqual([r['row'] for r in second],[0]);d.decode(av.Packet(base64.b64decode(second[0]['data_base64'])))
 def test_bounds(self):
  e=s.StripeEncoder()
  for width,height in [(127,80),(128,79),(2562,80)]:
   with self.assertRaises(ValueError):e.encode(bytes(width*height*4),width,height,8000000)
if __name__=='__main__':unittest.main()
