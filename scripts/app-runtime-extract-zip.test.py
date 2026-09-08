#!/usr/bin/env python3
import hashlib
import importlib.util
import io
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile

sys.dont_write_bytecode = True
SOURCE = Path(__file__).with_name("app-runtime-extract-zip.py")
spec = importlib.util.spec_from_file_location("runtime_zip", SOURCE)
runtime_zip = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime_zip)


class NonSeekable(io.BytesIO):
    def seekable(self):
        return False

    def seek(self, *_args):
        raise OSError("stream")


class Extraction(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="chariox-native-zip-")
        self.root = Path(self.temporary.name).resolve()
        self.archive = self.root / "artifact.zip"
        self.destination = self.root / "extracted"

    def tearDown(self):
        self.temporary.cleanup()

    def make_zip(self, names=None, compression=zipfile.ZIP_DEFLATED, stream=False, link=False):
        output = NonSeekable() if stream else io.BytesIO()
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(output, "w", compression=compression) as archive:
                for name in names if names is not None else sorted(runtime_zip.FILES):
                    if link:
                        info = zipfile.ZipInfo(name)
                        info.create_system = 3
                        info.external_attr = (stat.S_IFLNK | 0o777) << 16
                        archive.writestr(info, b"elsewhere")
                    else:
                        archive.writestr(name, f"tiny inert fixture: {name}\n".encode())
        return bytearray(output.getvalue())

    def extract(self, data, digest=None):
        self.archive.write_bytes(data)
        runtime_zip.extract(self.archive, self.destination, digest or hashlib.sha256(data).hexdigest())

    def reject(self, data):
        with self.assertRaises(Exception):
            self.extract(data)
        self.assertFalse(self.destination.exists())

    def test_stored_deflated_and_streaming_descriptors(self):
        for method, stream in [(zipfile.ZIP_STORED, False), (zipfile.ZIP_DEFLATED, False), (zipfile.ZIP_DEFLATED, True)]:
            with self.subTest(method=method, stream=stream):
                self.extract(self.make_zip(compression=method, stream=stream))
                self.assertEqual({path.name for path in self.destination.iterdir()}, runtime_zip.FILES)
                for path in self.destination.iterdir():
                    self.assertEqual(path.read_bytes(), f"tiny inert fixture: {path.name}\n".encode())
                    self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
                    path.unlink()
                self.destination.rmdir()

    def test_exact_four_regular_files_and_supported_compression(self):
        names = sorted(runtime_zip.FILES)
        for changed in [names[:-1], names + ["extra"], names[:-1] + [names[0]],
                        names[:-1] + ["../escape"], names[:-1] + ["directory/"],
                        names[:-1] + ["/absolute"], names[:-1] + ["folder\\escape"]]:
            self.reject(self.make_zip(names=changed))
        self.reject(self.make_zip(link=True))
        self.reject(self.make_zip(compression=zipfile.ZIP_BZIP2))

    def test_header_admission_rejects_large_directory_multidisk_and_zip64(self):
        for relative, format_, value in [(4, "H", 1), (8, "H", 65535), (10, "H", 65535),
                                         (12, "I", 0xFFFFFFFF), (16, "I", 0xFFFFFFFF), (20, "H", 1)]:
            data = self.make_zip()
            struct.pack_into("<" + format_, data, len(data) - 22 + relative, value)
            self.reject(data)
        self.reject(self.make_zip() + b"unaccounted trailing data")

    def test_invalid_directory_is_rejected_before_zipfile_allocation(self):
        data = self.make_zip()
        struct.pack_into("<I", data, len(data) - 22 + 12, runtime_zip.MAX_DIRECTORY + 1)
        with patch.object(runtime_zip.zipfile, "ZipFile") as parser:
            self.reject(data)
            parser.assert_not_called()
        with patch.object(runtime_zip, "MAX_SMALL", 8):
            self.reject(self.make_zip())

    def test_encryption_oversize_and_local_central_disagreement(self):
        for relative, format_, value in [(8, "H", 1), (20, "I", runtime_zip.MAX_TOTAL + 1),
                                         (28, "H", 65535), (42, "I", 1)]:
            data = self.make_zip()
            central = data.index(b"PK\x01\x02")
            struct.pack_into("<" + format_, data, central + relative, value)
            self.reject(data)
        data = self.make_zip()
        data[30] ^= 1
        self.reject(data)

    def test_crc_failure_cleans_partial_output(self):
        data = self.make_zip(compression=zipfile.ZIP_STORED)
        name_length, extra_length = struct.unpack_from("<HH", data, 26)
        data[30 + name_length + extra_length] ^= 1
        self.reject(data)

    def test_sha_mismatch_existing_destination_and_links_are_preserved(self):
        data = self.make_zip()
        with self.assertRaises(Exception):
            self.extract(data, "0" * 64)
        self.assertFalse(self.destination.exists())
        self.destination.mkdir(mode=0o700)
        keep = self.destination / "keep"
        keep.write_bytes(b"unrelated")
        with self.assertRaises(Exception):
            self.extract(data)
        self.assertEqual(keep.read_bytes(), b"unrelated")
        self.archive.unlink()
        self.archive.symlink_to(keep)
        with self.assertRaises(Exception):
            runtime_zip.extract(self.archive, self.root / "other", "0" * 64)
        self.assertEqual(keep.read_bytes(), b"unrelated")

    def test_repository_and_nonprivate_parent_rejected(self):
        data = self.make_zip()
        (self.root / ".git").write_text("gitdir: deliberately-not-read")
        self.reject(data)
        (self.root / ".git").unlink()
        self.root.chmod(0o755)
        try:
            self.reject(data)
        finally:
            self.root.chmod(0o700)

    def test_cli_failure_has_one_stable_error_without_paths(self):
        result = subprocess.run([sys.executable, str(SOURCE), "--unknown-secret-path"], capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"app_runtime_artifact_zip_rejected\n")


if __name__ == "__main__":
    unittest.main()
