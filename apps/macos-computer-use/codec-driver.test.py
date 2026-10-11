"""Run the real encoder/decoder driver from a fresh evidence directory."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class FreshCodecEvidence(unittest.TestCase):
    def test_fresh_evidence_root_reaches_repair_fallback_and_independent_decode(self):
        build_parent = Path(os.environ.get('CUMAC_BUILD_DIR',
            '/Users/miguel/.chariox/dev/cumac/build/codec-driver-tests'))
        evidence_parent = Path(os.environ.get('CUMAC_EVIDENCE_DIR',
            '/Users/miguel/.codex/evidence/browser-computer-use-macos/codec-driver-tests'))
        build_parent.mkdir(parents=True, exist_ok=True)
        evidence_parent.mkdir(parents=True, exist_ok=True)
        build = Path(tempfile.mkdtemp(prefix='codec-', dir=build_parent))
        evidence = Path(tempfile.mkdtemp(prefix='codec-', dir=evidence_parent)) / 'fresh-artifacts'
        self.assertFalse(evidence.exists())
        driver = os.environ.get('CUMAC_CODEC_DRIVER', str(Path(__file__).with_name('check-codec.sh')))
        env = dict(os.environ, CUMAC_BUILD_DIR=str(build), CUMAC_EVIDENCE_DIR=str(evidence))
        result = subprocess.run(['bash', driver], env=env, text=True, capture_output=True)
        print(result.stdout, result.stderr, flush=True)
        self.assertEqual(result.returncode, 0, 'fresh-root codec driver must finish every regression and independent decode')
        self.assertGreater((evidence / 'public-protected.h264').stat().st_size, 0)
        self.assertTrue((build / 'codec-tests').is_file())
        self.assertTrue((build / 'codec-decode-tests').is_file())
        self.assertIn('quiet source, settled exact repair and PNG fallback', result.stdout)
        self.assertIn('independent software H264 decode', result.stdout)
        print(f'PASS fresh codec artifact and independent decode at {evidence}', flush=True)


if __name__ == '__main__':
    unittest.main()
