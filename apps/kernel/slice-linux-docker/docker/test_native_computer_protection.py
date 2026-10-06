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
if __name__=='__main__':unittest.main()
