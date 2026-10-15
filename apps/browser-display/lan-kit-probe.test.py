# MP-08/MP-10/MP-11: packaging must exercise the dlopened codec in isolation.
import os,runpy,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
class IsolatedCodec(unittest.TestCase):
 def test_native_kit_runs_codec_probe_inside_chroot(self):
  with tempfile.TemporaryDirectory() as root:
   kit=Path(root);(kit/'runtime').mkdir();(kit/'runtime/native-worker').touch()
   with patch.object(sys,'argv',['check-lan-kit.py',root]),patch('subprocess.run') as run:
    runpy.run_path(str(Path(__file__).with_name('check-lan-kit.py')),run_name='__main__')
   probes=[call for call in run.call_args_list if '--display-native-codec-probe' in call.args[0]]
   self.assertEqual(len(probes),1,'MP-10: isolated check must open native encoder and decoder')
   self.assertIn('/runtime/native-worker',probes[0].args[0]);self.assertTrue(probes[0].kwargs['check'])
if __name__=='__main__':unittest.main()
