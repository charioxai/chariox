"""The release regression must fail closed at either build or execution."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('release-session-project.sh')


class ReleasePhases(unittest.TestCase):
    def run_phases(self, compile_status=0, test_status=0):
        with tempfile.TemporaryDirectory(prefix='release-phases-') as directory:
            root = Path(directory)
            cargo = root / 'cargo'
            cargo.write_text('''#!/usr/bin/env python3
import json, os, sys
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(json.dumps(sys.argv[1:]) + '\\n')
sys.exit(int(os.environ['COMPILE_STATUS' if '--no-run' in sys.argv else 'TEST_STATUS']))
''')
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=f'{root}:'+os.environ['PATH'],
                       CALLS=str(root / 'calls'), COMPILE_STATUS=str(compile_status),
                       TEST_STATUS=str(test_status))
            result = subprocess.run(['bash', str(SCRIPT)], env=env, capture_output=True, text=True)
            import json
            calls = [json.loads(line) for line in (root / 'calls').read_text().splitlines()]
            return result, calls

    def test_compile_failure_stops_before_execution(self):
        result, calls = self.run_phases(compile_status=17)
        self.assertEqual(result.returncode, 17)
        self.assertEqual(len(calls), 1)
        self.assertIn('compile finished: status=17', result.stdout)
        self.assertNotIn('test started', result.stdout)

    def test_execution_failure_is_reported(self):
        result, calls = self.run_phases(test_status=19)
        self.assertEqual(result.returncode, 19)
        self.assertEqual(len(calls), 2)
        self.assertIn('test finished: status=19', result.stdout)

    def test_success_compiles_then_runs_the_exact_release_regression(self):
        result, calls = self.run_phases()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 2)
        self.assertIn('--no-run', calls[0])
        self.assertNotIn('--no-run', calls[1])
        self.assertEqual(calls[0][:-1], calls[1][:len(calls[0])-1])
        self.assertEqual(calls[1][-4:], ['--', '--exact', 'visible_session_project_assignment_survives_release_builds', '--nocapture'])
        self.assertIn('--release', calls[0])
        self.assertIn('session_project_release', calls[0])
        self.assertNotIn('--lib', calls[0])
        self.assertIn('compile finished: status=0', result.stdout)
        self.assertIn('test finished: status=0', result.stdout)


if __name__ == '__main__':
    unittest.main()
