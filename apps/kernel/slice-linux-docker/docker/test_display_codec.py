"""MP-11: public codec cache/hash checks, supplementary to live capture."""
import bz2
import hashlib
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("display_codec", Path(__file__).with_name("install-display-codec.py"))
codec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(codec)

class DisplayCodecTest(unittest.TestCase):
    def pin(self, library=b"public codec fixture"):
        data = bz2.compress(library)
        return data, {"compressed_sha256": hashlib.sha256(data).hexdigest(), "library_sha256": hashlib.sha256(library).hexdigest(), "url": "https://ciscobinary.openh264.org/libopenh264-test.so.bz2"}

    def test_both_hashes_and_bounded_archive_are_required(self):
        data, pin = self.pin()
        self.assertEqual(codec.checked_library(data, pin), b"public codec fixture")
        with self.assertRaises(ValueError):
            codec.checked_library(data + b"changed", pin)
        with self.assertRaises(ValueError):
            codec.checked_library(data, {**pin, "library_sha256": "0" * 64})
        data, pin = self.pin(b"x" * (codec.MAX_LIBRARY + 1))
        with self.assertRaises(ValueError):
            codec.checked_library(data, pin)
        data, pin = self.pin()
        data += b"trailing"
        with self.assertRaises(ValueError):
            codec.checked_library(data, {**pin, "compressed_sha256": hashlib.sha256(data).hexdigest()})

    def test_valid_private_cache_needs_no_network(self):
        with tempfile.TemporaryDirectory(prefix="mp11-codec-") as directory:
            root = Path(directory)
            data, pin = self.pin()
            library = root / "libopenh264.so.8"
            library.write_bytes(b"public codec fixture")
            library.chmod(0o644)
            self.assertEqual(codec.install(root, pin), library)

    def test_world_writable_library_cannot_be_loaded_even_with_correct_hash(self):
        with tempfile.TemporaryDirectory(prefix="mp11-codec-") as directory:
            root = Path(directory)
            _, pin = self.pin()
            library = root / "libopenh264.so.8"
            library.write_bytes(b"public codec fixture")
            library.chmod(0o666)
            with self.assertRaises(ValueError):
                codec.install(root, pin)

    def test_symlink_or_shared_file_never_becomes_the_library(self):
        with tempfile.TemporaryDirectory(prefix="mp11-codec-") as directory:
            root = Path(directory)
            _, pin = self.pin()
            other = root / "public-other"
            other.write_bytes(b"public codec fixture")
            library = root / "libopenh264.so.8"
            library.symlink_to(other)
            with self.assertRaises(OSError):
                codec.install(root, pin)
            library.unlink()
            os.link(other, library)
            with self.assertRaises(ValueError):
                codec.install(root, pin)

    def test_foreign_writable_or_symlink_cache_is_refused(self):
        with tempfile.TemporaryDirectory(prefix="mp11-codec-") as directory:
            root = Path(directory)
            _, pin = self.pin()
            cache = root / "cache"
            cache.mkdir(mode=0o777)
            cache.chmod(0o777)
            with self.assertRaises(ValueError):
                codec.install(cache, pin)
            link = root / "alias"
            link.symlink_to(root, target_is_directory=True)
            with self.assertRaises(ValueError):
                codec.install(link, pin)

if __name__ == "__main__":
    unittest.main()
