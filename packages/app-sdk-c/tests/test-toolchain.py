"""Pin and extraction boundary checks; no toolchain/network is needed."""
import hashlib
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import toolchain


class ToolchainTests(unittest.TestCase):
    def test_checksums_before_extraction_and_marks_only_a_complete_install(self):
        with tempfile.TemporaryDirectory(prefix="chariox-c-pins-") as temporary:
            root = Path(temporary)
            archive = root / "fixture.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                member = tarfile.TarInfo("fixture/tool")
                member.size = 3
                tar.addfile(member, io.BytesIO(b"abc"))
            pin = {"url": archive.as_uri(), "sha256": "0" * 64, "directory": "fixture"}
            target = root / "bad"
            with patch.object(toolchain, "assets", return_value=[pin]):
                with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                    toolchain.install(target)
            self.assertFalse((target / "fixture").exists())
            self.assertFalse((target / "toolchain.lock.json").exists())
            pin["sha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
            target = root / "good"
            with patch.object(toolchain, "assets", return_value=[pin]):
                toolchain.install(target)
            self.assertEqual((target / "fixture/tool").read_bytes(), b"abc")
            self.assertEqual((target / "toolchain.lock.json").read_bytes(), toolchain.LOCK.read_bytes())
            with self.assertRaisesRegex(ValueError, "empty"):
                toolchain.install(target)

    def test_stale_pins_and_unsupported_hosts_fail_explicitly(self):
        with tempfile.TemporaryDirectory(prefix="chariox-c-pins-") as temporary:
            root = Path(temporary)
            (root / "toolchain.lock.json").write_text("{}")
            with self.assertRaisesRegex(ValueError, "pins changed"):
                toolchain.tools(root)
        with patch.object(toolchain.platform, "machine", return_value="unknown"):
            with self.assertRaisesRegex(ValueError, "unsupported build host"):
                toolchain.assets()


if __name__ == "__main__":
    unittest.main()
