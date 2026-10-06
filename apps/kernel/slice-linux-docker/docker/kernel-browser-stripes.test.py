"""MP-08/MP-10/MP-11: real codec row references, idle and collision checks."""
import base64,importlib.util,unittest,sys,os
from pathlib import Path
import av
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('stripes',Path(os.environ.get('MP_STRIPE_MODULE',Path(__file__).with_name('kernel-browser-stripes.py'))))
s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)
class StripeTests(unittest.TestCase):
 def test_mp10_unchanged_rows_skip_raster_slices_and_release_mapping(self):
  import mmap
  class Raster(bytearray):
   slices=0
   def __getitem__(self,key):
    if isinstance(key,slice):self.slices+=1
    return super().__getitem__(key)
  e=s.StripeEncoder();pixels=Raster(128*128*4)
  try:
   e.encode(pixels,128,128,8000000);pixels.slices=0
   self.assertEqual(e.encode(pixels,128,128,8000000),[])
   self.assertEqual(pixels.slices,0,'unchanged rows must compare admitted buffers without allocating row slices')
   with mmap.mmap(-1,len(pixels)) as shared:
    self.assertEqual(e.encode(shared,128,128,8000000),[])
    shared[0]=255
    self.assertEqual([r['row'] for r in e.encode(shared,128,128,8000000)],[0])
   # Exiting the context must not leave a ctypes/memoryview buffer export.
  finally:e.close()
 def test_mp10_direct_bgrx_conversion_channel_order_stride_and_fallback(self):
  # MP-08/MP-10: compare actual SIMD planes with the portable converter.
  import os
  library=os.environ.get('CHARIOX_BROWSER_DISPLAY_LIBYUV')
  if not library:self.skipTest('explicit libyuv path required for SIMD comparison')
  fast=s.BgrConverter(library);portable=s.BgrConverter(False)
  for colour in [(0,0,0,0),(255,255,255,0),(255,0,0,0),(0,255,0,0),(0,0,255,0)]:
   data=bytes(colour)*(130*18)
   a=fast.convert(data,130,18);b=portable.convert(data,130,18)
   for pa,pb in zip(a.planes,b.planes):
    for y in range(pa.height):
     aa=bytes(pa)[y*pa.line_size:y*pa.line_size+pa.width]
     bb=bytes(pb)[y*pb.line_size:y*pb.line_size+pb.width]
     self.assertTrue(all(abs(x-y)<=1 for x,y in zip(aa,bb)),colour)
  with self.assertRaises(ValueError):fast.convert(b'bad',130,18)

 def test_mp10_pool_preserves_serial_bytes_references_and_idle_skip(self):
  from unittest.mock import patch
  outputs=[]
  for workers in ('1','2','4'):
   with patch.dict(os.environ,{'CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS':workers}):encoder=s.StripeEncoder()
   try:
    self.assertEqual(encoder.workers,int(workers))
    pixels=bytearray(128*128*4);rows=[]
    for n in range(12):
     pixels[(n%8)*16*128*4+4]=n*17;rows.append(encoder.encode(pixels,128,128,8000000,n==0))
    self.assertEqual(encoder.encode(pixels,128,128,8000000),[]);outputs.append(rows)
   finally:encoder.close()
  self.assertEqual(outputs[0],outputs[1]);self.assertEqual(outputs[0],outputs[2])
  with patch.dict(os.environ,{'CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS':'32'}):
   with self.assertRaises(ValueError):s.StripeEncoder()

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
