# MP-08/MP-10/MP-11: public runtime-only library roots, no provider state.
import unittest
from pathlib import Path
from unittest.mock import patch
from subprocess import CompletedProcess
from lan_codec_runtime import codec_roots
class CodecRoots(unittest.TestCase):
 def test_absolute_openh264_and_exact_decoder_abi(self):
  with patch('subprocess.run',return_value=CompletedProcess([],0,'MP-08/MP-10: native decoder libavcodec.so.62\n','')),patch('subprocess.check_output',return_value=' libavcodec.so.62 (libc6) => /public/libavcodec.so.62\n'),patch.object(Path,'is_file',return_value=True):
   self.assertEqual(codec_roots('/public/worker',{'CHARIOX_BROWSER_DISPLAY_OPENH264':'/public/cisco.so'}),[(Path('/public/libavcodec.so.62'),'libavcodec.so.62'),(Path('/public/cisco.so'),'libopenh264.so.8')])
 def test_default_does_not_bundle_x264(self):
  with patch('subprocess.run',return_value=CompletedProcess([],0,'MP-08/MP-10: native decoder libavcodec.so.62\n','')),patch('subprocess.check_output',return_value=' libavcodec.so.62 (libc6) => /public/av.so\n libopenh264.so.8 (libc6) => /public/openh264.so\n libx264.so.165 (libc6) => /public/x264.so\n'),patch.object(Path,'is_file',return_value=True):
   self.assertEqual([name for _,name in codec_roots('/public/worker',{})],['libavcodec.so.62','libopenh264.so.8'])
 def test_missing_dependency_refuses_packaging(self):
  with patch('subprocess.run',return_value=CompletedProcess([],0,'MP-08/MP-10: native decoder libavcodec.so.62\n','')),patch('subprocess.check_output',return_value=''):
   with self.assertRaisesRegex(RuntimeError,'missing codec library'):codec_roots('/public/worker',{})
if __name__=='__main__':unittest.main()
