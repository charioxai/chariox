"""MP-08/MP-11: native field masks and clipboard capture fences."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('native_computer',Path(__file__).with_name('native-computer.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
clipboard=module.load('native-clipboard')
x11=module.load('native-x11')
# Real X11 opener (patched Display); every other helper is the given fake.
helpers=lambda accessibility:(lambda name: accessibility if name=='native-accessibility' else x11 if name=='native-x11' else clipboard)
class ProtectionTests(unittest.TestCase):
    def test_mp08_mp11_password_dots_leave_desktop_visible_and_only_plaintext_value_is_masked(self):
        # Password roles still protect structured text/input; their dots are safe
        # pixels. A registered value shown in an ordinary entry is a local mask.
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        for masks in ([],[[2,1,2,2]]):
            with self.subTest(masks=masks):
                tree={'available':True,'complete':True,'protected':True,
                      'nodes':[{'role':'password text','protected':True,'name':'[protected]'}],
                      'masks':masks,'uncovered':[]}
                with patch.object(module,'load',side_effect=helpers(SimpleNamespace(snapshot=lambda *args:tree))),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
                    result=module.main({'op':'screenshot','mask':False,'processes':[],'values':['synthetic-only']})
                self.assertFalse(result['protected'])
                with module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64']))) as image:
                    for y in range(4):
                        for x in range(8):
                            covered=bool(masks) and 2<=x<4 and 1<=y<3
                            self.assertEqual(image.getpixel((x,y)),(0,0,0) if covered else (200,200,200))

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
        with patch.object(module,'load',side_effect=helpers(accessibility)),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
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
        with patch.object(module,'load',side_effect=helpers(SimpleNamespace(snapshot=lambda *args:tree))),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[]})
            image=module.Image.open(module.io.BytesIO(module.base64.b64decode(result['data_base64'])))
            self.assertEqual(image.getpixel((4,2)),(0,0,0))
    def test_mp08_mp11_browser_protection_fences_both_snapshots_and_reports_withheld_windows(self):
        calls=[]
        def snapshot(*args):
            calls.append(args)
            return {'available':True,'complete':True,'protected':False,'nodes':[],'masks':[[0,0,2,2]],'browser_withheld':1}
        raw=SimpleNamespace(depth=24,data=bytes([200,200,200,0])*8*4)
        screen=SimpleNamespace(width_in_pixels=8,height_in_pixels=4,root=SimpleNamespace(get_image=lambda *args:raw))
        protection={'pages':[]}
        with patch.object(module,'load',side_effect=helpers(SimpleNamespace(snapshot=snapshot))),patch.object(module.display,'Display',return_value=SimpleNamespace(screen=lambda:screen,close=lambda:None)):
            result=module.main({'op':'screenshot','mask':False,'processes':[],'browser_protection':protection})
        self.assertEqual([call[2] for call in calls],[protection,protection])
        self.assertEqual(result['browser_withheld'],1)
if __name__=='__main__':unittest.main()
