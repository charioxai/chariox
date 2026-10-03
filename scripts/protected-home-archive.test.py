#!/usr/bin/env python3
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tarfile
import unittest

path = Path(__file__).resolve().parent.parent / "apps/kernel/slice-linux-docker/validate-home-archive.py"
spec = importlib.util.spec_from_file_location("validator", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def member(name, kind=tarfile.REGTYPE, link=""):
    result = tarfile.TarInfo(name)
    result.type = kind
    result.linkname = link
    return result


class ArchiveMetadataTests(unittest.TestCase):
    def test_ordinary_browser_and_full_home_metadata(self):
        module.validate_members([member(".", tarfile.DIRTYPE), member(".chariox/browser/Cookies"), member("Downloads/synthetic"), member("notes", tarfile.SYMTYPE, "Downloads/synthetic")])

    def test_private_roots_and_escapes_refuse(self):
        for name in [".codex/auth.json", ".local/share/pki/nssdb/key4.db", ".chariox/state/daemon/identity.json", "../private", "/private"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                module.validate_members([member(name)])

    def test_links_cannot_target_private_or_escape(self):
        for kind, target in [(tarfile.SYMTYPE, "/var/lib/chariox/slice-private/kernel"), (tarfile.SYMTYPE, "../private"), (tarfile.LNKTYPE, ".codex/auth.json")]:
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                module.validate_members([member("synthetic", kind, target)])

    def test_device_fifo_duplicate_and_setuid_refuse(self):
        unsafe = member("synthetic"); unsafe.mode = 0o4755
        for members in [[member("synthetic", tarfile.CHRTYPE)], [member("synthetic", tarfile.FIFOTYPE)], [member("duplicate"), member("duplicate")], [unsafe]]:
            with self.assertRaises(ValueError):
                module.validate_members(members)


class ArchiveStreamTests(unittest.TestCase):
    def test_validator_reads_its_input_to_eof(self):
        # The broker pipes the decoder into the validator and fails if the
        # validator closes its input first, so a valid archive must be drained.
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w") as writer:
            entry = tarfile.TarInfo("synthetic")
            entry.size = 9
            writer.addfile(entry, io.BytesIO(b"synthetic"))
        stream = archive.getvalue() + bytes(1 << 20)
        process = subprocess.Popen([sys.executable, str(path)], stdin=subprocess.PIPE,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for offset in range(0, len(stream), 1 << 16):
                process.stdin.write(stream[offset:offset + (1 << 16)])
            process.stdin.close()
        except BrokenPipeError:
            self.fail("validator closed its input before EOF")
        finally:
            process.wait(timeout=30)
        self.assertEqual(process.returncode, 0)


if __name__ == "__main__":
    unittest.main()
