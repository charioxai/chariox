"""MP-11: clipboard reads must fence native protection changes during the read."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('native_computer',Path(__file__).with_name('native-computer.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class ProtectionTests(unittest.TestCase):
    def test_clipboard_is_withheld_when_native_coverage_changes_during_read(self):
        before={'available':True,'complete':True,'protected':False,'nodes':[]}
        after={**before,'protected':True}
        accessibility=SimpleNamespace(snapshot=unittest.mock.Mock(side_effect=[before,after]))
        with patch.object(module,'load',return_value=accessibility),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'synthetic-private-canary')):
            with self.assertRaises(ValueError):module.main({'op':'clipboard_read','mask':False,'processes':[]})
    def test_unchanged_complete_coverage_allows_public_clipboard(self):
        before={'available':True,'complete':True,'protected':False,'nodes':[]}
        accessibility=SimpleNamespace(snapshot=lambda _:before)
        with patch.object(module,'load',return_value=accessibility),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'public')):
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'public'})
    def test_mp08_uncovered_owned_window_is_blacked_out_not_the_desktop(self):
        tree={'available':True,'complete':True,'protected':False,'nodes':[],'uncovered':[[2,1,3,2]]}
        accessibility=SimpleNamespace(snapshot=lambda _:tree)
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        with patch.object(module,'load',return_value=accessibility),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            self.assertFalse(result['protected'])
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((2,1)),(0,0,0));self.assertEqual(image.getpixel((4,2)),(0,0,0))
            self.assertEqual(image.getpixel((1,1)),(200,200,200));self.assertEqual(image.getpixel((5,1)),(200,200,200));self.assertEqual(image.getpixel((2,3)),(200,200,200))
            # A terminal selection may hold typed secrets: keep the clipboard closed.
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
if __name__=='__main__':unittest.main()
