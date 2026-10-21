#!/usr/bin/env python3
"""MP-08 / MP-11: the default drill bundle copies every helper its modules load."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('drill_helper_closure', HERE / 'drill-helper-closure.py')
closure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(closure)


class DrillHelperClosure(unittest.TestCase):
    def test_default_bundle_has_native_computer_and_accessibility_dependencies(self):
        bundle = closure.default_helpers(HERE / 'docker')
        for name in ('native-fill-targets.py', 'native-input-destination.py', 'desktop-applications.py'):
            self.assertIn(name, bundle)

    def test_no_copied_helper_references_a_missing_sibling(self):
        docker = HERE / 'docker'
        bundle = set(closure.default_helpers(docker))
        for name in bundle:
            self.assertLessEqual(closure.references(docker, name), bundle, name)

    def test_follows_load_and_file_name_references(self):
        with tempfile.TemporaryDirectory() as root:
            docker = Path(root)
            (docker / 'native-computer.py').write_text("load('native-input-destination')\n")
            (docker / 'native-input-destination.py').write_text("Path(__file__).with_name('native-fill-targets.py')\n")
            (docker / 'native-fill-targets.py').write_text('')
            (docker / 'unrelated.py').write_text('')
            self.assertEqual(closure.default_helpers(docker), ['native-computer.py', 'native-fill-targets.py', 'native-input-destination.py'])


if __name__ == '__main__':
    unittest.main()
