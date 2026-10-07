"""MP-08 / MP-10 / MP-11: artifact and process admission, without provider execution."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from turn import preflight, signal_owned_descendant


class PreflightTests(unittest.TestCase):
    def test_digest_mismatch_stops_before_executing_artifact(self):
        with tempfile.TemporaryDirectory(prefix='chariox-evals-test-') as tmp:
            root = Path(tmp); (root / 'bin').mkdir()
            (root / 'bin/chariox-kernel').write_bytes(b'not an executable')
            with patch('turn.subprocess.check_output') as execute:
                with self.assertRaises(ValueError):
                    preflight(root, 'a' * 40, 'b' * 64, 435)
                execute.assert_not_called()

    def test_client_entry_must_be_in_hashed_bundle(self):
        with tempfile.TemporaryDirectory(prefix='chariox-evals-test-') as tmp:
            root = Path(tmp); (root / 'bin').mkdir()
            data = b'fixture kernel'; (root / 'bin/chariox-kernel').write_bytes(data)
            digest = hashlib.sha256(data).hexdigest()
            (root / 'eval-runtime.json').write_text(json.dumps({'source_commit': 'a' * 40,
                                                             'kernel_sha256': digest, 'files': {'bin/chariox-kernel': digest}}))
            with patch('turn.subprocess.check_output', return_value='435\n'):
                with self.assertRaisesRegex(ValueError, 'incomplete real-client runtime manifest'):
                    preflight(root, 'a' * 40, digest, 435)

    def test_pid_guard_precedes_any_signal_or_pidfd(self):
        for pid in [0, 1, -1, None, float('nan')]:
            with patch('turn.os.pidfd_open') as open_pidfd:
                with self.assertRaises(ValueError):
                    signal_owned_descendant(pid, 'birth', 15)
                open_pidfd.assert_not_called()

    def test_reused_process_identity_receives_no_signal(self):
        with patch('turn.os.pidfd_open', return_value=99), patch('turn.os.close') as close:
            with patch('turn.process_identity', return_value='new'), patch('turn.signal.pidfd_send_signal') as send:
                signal_owned_descendant(22, 'old', 15)
                send.assert_not_called(); close.assert_called_once_with(99)


if __name__ == '__main__':
    unittest.main()
