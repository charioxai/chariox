"""Exercise the unsigned owner driver with real policy code and Mach-O signing.

Bundle compiles use a tiny inert program so no helper is built or launched.
The production driver still supplies all compiler/linker flags. Outputs stay
under the approved build root and are retained for inspection.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class UnsignedOwnerBuild(unittest.TestCase):
    def test_policy_runs_signed_and_release_executables_stay_unsigned(self):
        parent = Path(os.environ.get('CUMAC_BUILD_DIR',
            '/Users/miguel/.chariox/dev/cumac/build/build-signing-tests'))
        parent.mkdir(parents=True, exist_ok=True)
        root = Path(tempfile.mkdtemp(prefix='unsigned-', dir=parent))
        tools = root / 'tools'
        tools.mkdir()
        inert = root / 'Inert.swift'
        inert.write_text('@main struct Inert { static func main() {} }\n')
        swiftc = shutil.which('swiftc')
        wrapper = tools / 'swiftc'
        wrapper.write_text('''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
output = args[args.index('-o') + 1]
if not output.endswith('/policy-tests'):
    args = [arg for arg in args if not arg.endswith('.swift')]
    args.insert(0, os.environ['CUMAC_INERT_SOURCE'])
sys.exit(subprocess.call([os.environ['CUMAC_REAL_SWIFTC'], *args]))
''')
        wrapper.chmod(0o700)
        build = root / 'release'
        env = dict(os.environ, CUMAC_UNSIGNED='1', CUMAC_BUILD_DIR=str(build),
            CUMAC_REAL_SWIFTC=swiftc, CUMAC_INERT_SOURCE=str(inert),
            PATH=str(tools) + os.pathsep + os.environ['PATH'])
        result = subprocess.run(['bash', str(Path(__file__).with_name('build.sh'))],
            env=env, text=True, capture_output=True)
        print(result.stdout, result.stderr, flush=True)
        self.assertEqual(result.returncode, 0, 'unsigned owner build must run policy tests and reach every bundle')
        policy = subprocess.run(['codesign', '-dv', str(build / 'policy-tests')],
            text=True, capture_output=True)
        self.assertEqual(policy.returncode, 0, policy.stderr)
        self.assertIn('Signature=adhoc', policy.stderr)
        for kind in ['Helper', 'Fixture', 'Cover']:
            binary = build / f'Chariox Computer {kind}.app' / 'Contents/MacOS' / kind.lower()
            self.assertTrue(binary.is_file())
            signature = subprocess.run(['codesign', '-dv', str(binary)],
                text=True, capture_output=True)
            self.assertNotEqual(signature.returncode, 0, f'{kind} must remain unsigned')
            self.assertIn('not signed at all', signature.stderr)
        repeat = subprocess.run(['bash', str(Path(__file__).with_name('build.sh'))],
            env=env, text=True, capture_output=True)
        self.assertNotEqual(repeat.returncode, 0)
        self.assertIn('fresh destination', repeat.stderr)
        print('PASS real signed policy execution, unsigned inert release binaries and fresh-path guard', flush=True)


if __name__ == '__main__':
    unittest.main()
