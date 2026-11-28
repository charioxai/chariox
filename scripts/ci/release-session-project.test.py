"""The release build fails closed and execution uses only its captured artifact."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('release-session-project.py')


class ReleasePhases(unittest.TestCase):
    def run_phases(self, compile_status=0, test_status=0, artifact=True):
        with tempfile.TemporaryDirectory(prefix='release-phases-') as directory:
            root = Path(directory)
            for name, source in {
                'cargo': '''import json, os, sys
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(json.dumps(['cargo', *sys.argv[1:]]) + '\\n')
if os.environ['ARTIFACT'] == '1':
    print(json.dumps({'reason': 'compiler-artifact', 'target': {'name': 'chariox_kernel_release_tests'}, 'profile': {'test': True}, 'executable': os.environ['TEST_BINARY']}))
sys.exit(int(os.environ['COMPILE_STATUS']))
''',
                'timeout': '''import json, os, sys
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(json.dumps(['timeout', *sys.argv[1:]]) + '\\n')
os.execv(sys.argv[3], sys.argv[3:])
''',
                'regression': '''import json, os, sys
with open(os.environ['CALLS'], 'a') as calls:
    calls.write(json.dumps(['regression', *sys.argv[1:]]) + '\\n')
sys.exit(int(os.environ['TEST_STATUS']))
''',
            }.items():
                path = root / name
                path.write_text('#!/usr/bin/env python3\n'+source)
                path.chmod(0o755)
            env = dict(os.environ, PATH=f'{root}:'+os.environ['PATH'],
                       CALLS=str(root / 'calls'), COMPILE_STATUS=str(compile_status),
                       TEST_STATUS=str(test_status), TEST_BINARY=str(root / 'regression'),
                       ARTIFACT='1' if artifact else '0')
            result = subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True)
            calls = [json.loads(line) for line in (root / 'calls').read_text().splitlines()]
            return result, calls

    def test_compile_failure_stops_before_execution(self):
        result, calls = self.run_phases(compile_status=17)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)
        self.assertIn('compile finished: status=17', result.stdout)
        self.assertNotIn('test started', result.stdout)

    def test_execution_failure_is_reported(self):
        result, calls = self.run_phases(test_status=19)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 3)
        self.assertIn('test finished: status=19', result.stdout)

    def test_success_runs_the_captured_binary_with_a_hard_deadline(self):
        result, calls = self.run_phases()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 3)
        self.assertEqual(calls[0][0:2], ['cargo', 'test'])
        self.assertIn('chariox-kernel-release-tests', calls[0])
        self.assertIn('--release', calls[0])
        self.assertIn('--no-run', calls[0])
        self.assertEqual(calls[1][:3], ['timeout', '--signal=KILL', '60'])
        self.assertEqual(calls[1][4:], calls[2][1:])
        self.assertEqual(calls[2][1:], ['--exact', 'tests::visible_session_project_assignment_survives_release_builds', '--nocapture'])
        self.assertEqual(sum(call[0] == 'cargo' for call in calls), 1)

    def test_missing_artifact_does_not_execute_a_test(self):
        result, calls = self.run_phases(artifact=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)
        self.assertIn('Expected one release regression executable, found 0', result.stderr)


if __name__ == '__main__':
    unittest.main()
