import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('licenses', Path(__file__).with_name('licenses.py'))
licenses = importlib.util.module_from_spec(spec)
spec.loader.exec_module(licenses)


class LicensePolicy(unittest.TestCase):
    def test_permissive_alternatives(self):
        for expression in ['MIT OR Apache-2.0', 'MIT/Apache-2.0', 'MPL-2.0', 'Apache-2.0 WITH LLVM-exception', '(MIT OR GPL-3.0) AND BSD-3-Clause']:
            self.assertTrue(licenses.permitted(expression), expression)

    def test_unapproved_obligations_and_malformed_expressions(self):
        for expression in [None, '', 'GPL-3.0', 'AGPL-3.0', 'MIT AND GPL-3.0', 'MIT OR', '(MIT', 'MIT WITH Unknown', 'MIT Apache-2.0']:
            self.assertFalse(licenses.permitted(expression), expression)


if __name__ == '__main__':
    unittest.main()
