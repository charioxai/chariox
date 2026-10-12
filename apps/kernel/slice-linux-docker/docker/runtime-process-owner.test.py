"""MP-08/MP-10/MP-11: runtime signals retain launch ownership."""
import json
import pathlib
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent))
from owned_process_signals import OwnedProcesses


class RuntimeSignalTests(unittest.TestCase):
    def test_runtime_never_signals_by_process_name(self):
        self.assertNotIn('pkill ', (ROOT / 'start-runtime.sh').read_text())

    def test_reserved_and_reused_launches_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            record = pathlib.Path(directory) / 'owner.json'
            for pid in (0, 1, -1):
                record.write_text(json.dumps({'pid': pid, 'start': '1'}))
                result = subprocess.run([sys.executable, str(ROOT / 'runtime-process-owner.py'), 'stop', '--record', str(record)], capture_output=True)
                self.assertEqual(result.returncode, 2)
            outsider = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(60)'], start_new_session=True)
            owner = OwnedProcesses()
            launch = owner.record(outsider.pid, fresh_launch=True)
            try:
                record.write_text(json.dumps({'pid': outsider.pid, 'start': '0'}))
                result = subprocess.run([sys.executable, str(ROOT / 'runtime-process-owner.py'), 'stop', '--record', str(record)], capture_output=True)
                self.assertEqual(result.returncode, 0)
                self.assertFalse(record.exists())
                self.assertIsNone(outsider.poll())
                record.write_text(json.dumps({'pid': outsider.pid, 'start': '0'}))
                result = subprocess.run([sys.executable, str(ROOT / 'runtime-process-owner.py'), 'run', '--record', str(record), '--', sys.executable, '-c', 'pass'], capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(record.exists())
                self.assertIsNone(outsider.poll())
            finally:
                owner.group(launch, signal.SIGTERM)
                outsider.wait(timeout=5)
                owner.close()

    def test_dead_record_allows_runtime_restart(self):
        with tempfile.TemporaryDirectory() as directory:
            record = pathlib.Path(directory) / 'kernel-owner.json'
            record.write_text(json.dumps({'pid': 2147483647, 'start': '1'}))
            result = subprocess.run([sys.executable, str(ROOT / 'runtime-process-owner.py'), 'run', '--record', str(record), '--', sys.executable, '-c', 'pass'], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(record.exists())

    def test_live_record_refuses_duplicate_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            record = pathlib.Path(directory) / 'owner.json'
            child = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(60)'], start_new_session=True)
            owner = OwnedProcesses()
            launch = owner.record(child.pid, fresh_launch=True)
            try:
                value = {'pid': launch.pid, 'start': launch.start}
                record.write_text(json.dumps(value))
                result = subprocess.run([sys.executable, str(ROOT / 'runtime-process-owner.py'), 'run', '--record', str(record), '--', sys.executable, '-c', 'pass'], capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(json.loads(record.read_text()), value)
                self.assertIsNone(child.poll())
            finally:
                owner.group(launch, signal.SIGTERM)
                child.wait(timeout=5)
                owner.close()

    def test_stopping_retained_runtime_preserves_unrelated_process(self):
        with tempfile.TemporaryDirectory() as directory:
            record = pathlib.Path(directory) / 'owner.json'
            outsider = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(60)'], start_new_session=True)
            owner = OwnedProcesses()
            outsider_launch = owner.record(outsider.pid, fresh_launch=True, watch=True)
            runtime = subprocess.Popen([sys.executable, str(ROOT / 'runtime-process-owner.py'), 'run', '--record', str(record), '--', sys.executable, '-c', 'import time;time.sleep(60)'], start_new_session=True)
            runtime_launch = owner.record(runtime.pid, fresh_launch=True, watch=True)
            try:
                for _ in range(100):
                    if record.exists() and record.stat().st_size:
                        break
                    time.sleep(.05)
                value = json.loads(record.read_text())
                self.assertGreater(value['pid'], 1)
                result = subprocess.run([sys.executable, str(ROOT / 'runtime-process-owner.py'), 'stop', '--record', str(record)], capture_output=True, timeout=40)
                self.assertEqual(result.returncode, 0, result.stderr)
                runtime.wait(timeout=10)
                self.assertFalse(record.exists())
                self.assertIsNone(outsider.poll())
            finally:
                for launch in (runtime_launch, outsider_launch):
                    for row in owner.refresh():
                        if owner.matches(row) and owner.owned[row['pid']][1] is launch:
                            owner.group(launch, signal.SIGTERM, row['pgid'])
                runtime.wait(timeout=10)
                outsider.wait(timeout=5)
                owner.close()


if __name__ == '__main__':
    unittest.main()
