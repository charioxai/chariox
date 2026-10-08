"""MP-11: clipboard reads must fence native protection changes during the read."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('native_computer',Path(__file__).with_name('native-computer.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
clipboard=module.load('native-clipboard')
class ProtectionTests(unittest.TestCase):
    def test_mp11_opaque_browser_keeps_unknown_clipboard_contents_withheld(self):
        tree={'available':True,'complete':True,'protected':False,
              'nodes':[{'protected':True,'name':'[protected]'}],'uncovered':[]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree)
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else clipboard),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'private-browser-canary')) as read:
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
        read.assert_not_called()

    def test_mp11_r2_unknown_clipboard_source_is_withheld_with_public_coverage(self):
        tree={'available':True,'complete':True,'protected':False,'nodes':[]}
        with patch.object(module,'load',side_effect=lambda name:SimpleNamespace(snapshot=lambda *args:tree) if name=='native-accessibility' else clipboard), patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'unknown-source-canary')):
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})

    def test_mp11_public_coverage_without_a_clipboard_source_remains_withheld(self):
        before={'available':True,'complete':True,'protected':False,'nodes':[]}
        accessibility=SimpleNamespace(snapshot=lambda *args:before)
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else clipboard),patch.object(module.subprocess,'run',return_value=SimpleNamespace(stdout=b'public')):
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
    def test_mp08_uncovered_owned_window_is_blacked_out_not_the_desktop(self):
        # Owned app stacked above covers x>=4 of the terminal frame: only the exposed part is blacked out.
        tree={'available':True,'complete':True,'protected':False,'nodes':[],'uncovered':[[2,1,3,2]],'masks':[[2,1,2,2]]}
        accessibility=SimpleNamespace(snapshot=lambda *args:tree)
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        with patch.object(module,'load',side_effect=lambda name: accessibility if name=='native-accessibility' else clipboard),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            self.assertFalse(result['protected'])
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((2,1)),(0,0,0));self.assertEqual(image.getpixel((3,2)),(0,0,0));self.assertEqual(image.getpixel((4,2)),(200,200,200))
            self.assertEqual(image.getpixel((1,1)),(200,200,200));self.assertEqual(image.getpixel((5,1)),(200,200,200));self.assertEqual(image.getpixel((2,3)),(200,200,200))
            # A terminal selection may hold typed secrets: keep the clipboard closed.
            self.assertEqual(module.main({'op':'clipboard_read','mask':False,'processes':[]}),{'text':'[protected]'})
    def test_mp11_missing_masks_falls_back_to_whole_uncovered_frames(self):
        tree={'available':True,'complete':True,'protected':False,'nodes':[],'uncovered':[[2,1,3,2]]}
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        with patch.object(module,'load',side_effect=lambda name:SimpleNamespace(snapshot=lambda *args:tree) if name=='native-accessibility' else clipboard),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((4,2)),(0,0,0))
if __name__=='__main__':unittest.main()
