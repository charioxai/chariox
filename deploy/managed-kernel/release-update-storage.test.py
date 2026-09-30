#!/usr/bin/env python3
"""Exact-attempt cleanup refuses uncertain ownership and shared storage."""
import importlib.util
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_update_storage", Path(__file__).with_name("release-update-storage.py"))
storage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(storage)
UPDATE_ID = "managed_release_update_11111111-1111-1111-1111-111111111111"
OTHER_UPDATE_ID = "managed_release_update_22222222-2222-2222-2222-222222222222"


class StorageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="chariox-release-storage-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve() / "updates"
        self.uid = os.geteuid()
        self.attempt = storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
        self.keep = self.attempt / "keep"
        self.keep.write_text("this attempt")

    def cleanup(self, update_id=UPDATE_ID):
        storage.cleanup(self.root, update_id, expected_uid=self.uid)

    def refused(self, pattern):
        with self.assertRaisesRegex((ValueError, OSError), pattern):
            self.cleanup()
        self.assertEqual(self.keep.read_text(), "this attempt")

    def test_preparation_persists_exact_private_identity(self):
        self.assertEqual(self.attempt, self.root / UPDATE_ID)
        self.assertEqual(stat.S_IMODE(self.attempt.stat().st_mode), 0o700)
        marker = self.attempt / storage.MARKER
        self.assertEqual(marker.read_text(), UPDATE_ID + "\n")
        self.assertEqual(stat.S_IMODE(marker.stat().st_mode), 0o600)
        self.assertEqual(marker.stat().st_uid, self.uid)

    def test_prepare_and_cleanup_only_replace_the_same_owned_attempt(self):
        other = storage.prepare(self.root, OTHER_UPDATE_ID, expected_uid=self.uid)
        (other / "keep").write_text("other attempt")
        unknown = self.root / ".release-extract-unknown"
        unknown.mkdir()
        (unknown / "keep").write_text("unknown scratch")
        prepared = storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
        self.assertFalse(self.keep.exists())
        self.assertEqual(prepared, self.attempt)
        self.cleanup()
        self.cleanup()  # Reconciliation can retry after successful deletion.
        self.assertFalse(self.attempt.exists())
        self.assertEqual((other / "keep").read_text(), "other attempt")
        self.assertEqual((unknown / "keep").read_text(), "unknown scratch")

    def test_missing_or_mismatched_ownership_marker_prevents_deletion(self):
        marker = self.attempt / storage.MARKER
        marker.unlink()
        self.refused("No such file")
        marker.write_text(OTHER_UPDATE_ID + "\n")
        marker.chmod(0o600)
        self.refused("ownership mismatch")

    def test_boundary_requires_mode_0700(self):
        for mode in [0o755, 0o750, 0o777]:
            with self.subTest(mode=oct(mode)):
                self.attempt.chmod(mode)
                self.refused("private|unsafe")
        self.attempt.chmod(0o700)

    def test_marker_requires_private_regular_file(self):
        marker = self.attempt / storage.MARKER
        marker.chmod(0o644)
        self.refused("unsafe.*marker")
        marker.chmod(0o600)
        marker.write_text("x" * 257)
        self.refused("unsafe.*marker")
        marker.unlink()
        outside = Path(self.temp.name) / "outside-marker"
        outside.write_text(UPDATE_ID + "\n")
        outside.chmod(0o600)
        marker.symlink_to(outside)
        self.refused("unsafe.*marker")
        self.assertEqual(outside.read_text(), UPDATE_ID + "\n")

    def test_foreign_directory_or_marker_owner_prevents_deletion(self):
        real_lstat = Path.lstat
        for target in [self.root, self.attempt, self.attempt / storage.MARKER]:
            def foreign_owner(path):
                value = real_lstat(path)
                if path == target:
                    fields = list(value)
                    fields[4] = self.uid + 1
                    return os.stat_result(fields)
                return value
            with self.subTest(target=target.name), patch.object(Path, "lstat", foreign_owner):
                self.refused("unsafe")

    def test_symlinked_boundary_and_root_alias_are_refused(self):
        self.cleanup()
        outside = Path(self.temp.name) / "outside"
        outside.mkdir()
        (outside / "keep").write_text("outside")
        self.attempt.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "unsafe.*directory"):
            self.cleanup()
        alias = Path(self.temp.name) / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlinks"):
            storage.cleanup(alias, UPDATE_ID, expected_uid=self.uid)
        self.assertEqual((outside / "keep").read_text(), "outside")

    def test_symlinked_ancestor_and_unowned_directory_are_refused(self):
        self.cleanup()
        self.attempt.mkdir(mode=0o700)
        self.keep.write_text("this attempt")
        with self.assertRaises(OSError):
            storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
        self.assertEqual(self.keep.read_text(), "this attempt")
        alias = Path(self.temp.name) / "ancestor-alias"
        alias.symlink_to(self.root.parent, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlinks"):
            storage.cleanup(alias / self.root.name, UPDATE_ID, expected_uid=self.uid)
        self.assertEqual(self.keep.read_text(), "this attempt")

    def test_relative_roots_and_path_injection_are_refused(self):
        with self.assertRaisesRegex(ValueError, "absolute"):
            storage.cleanup(Path("relative"), UPDATE_ID, expected_uid=self.uid)
        for identity in ["../escape", "/absolute", UPDATE_ID + "/child", "managed_release_update_" + "g" * 36]:
            with self.subTest(identity=identity), self.assertRaisesRegex(ValueError, "identity"):
                self.cleanup(identity)
        self.assertEqual(self.keep.read_text(), "this attempt")

    def test_owned_symlinks_are_unlinked_without_following_their_targets(self):
        outside = Path(self.temp.name) / "outside-file"
        outside.write_text("outside")
        (self.attempt / "link").symlink_to(outside)
        self.cleanup()
        self.assertFalse(self.attempt.exists())
        self.assertEqual(outside.read_text(), "outside")

    @unittest.skipUnless(os.name == "posix", "requires FIFO")
    def test_special_files_prevent_cleanup_before_any_owned_file_is_removed(self):
        os.mkfifo(self.attempt / "pipe", 0o600)
        self.refused("special file")
        self.assertTrue((self.attempt / storage.MARKER).exists())

    def test_boundary_and_same_device_nested_mounts_are_refused(self):
        real_ismount = storage.os.path.ismount
        with patch.object(storage.os.path, "ismount", side_effect=lambda path: Path(path) == self.attempt or real_ismount(path)):
            self.refused("mounted.*boundary")
        child = self.attempt / "nested"
        child.mkdir()
        with patch.object(storage.os.path, "ismount", side_effect=lambda path: Path(path) == child or real_ismount(path)):
            self.refused("mounted content")

    def test_mount_inventory_detects_bind_mounts_even_when_ismount_does_not(self):
        mountinfo = Path("/proc/self/mountinfo")
        real_exists = Path.exists
        real_read_text = Path.read_text
        for suffix in ["/nested", "/space in mount"]:
            mount = str(self.attempt) + suffix
            escaped = mount.replace(" ", "\\040")
            line = "42 41 0:1 / " + escaped + " rw - tmpfs fixture rw\n"
            def exists(path):
                return True if path == mountinfo else real_exists(path)
            def read_text(path, *args, **kwargs):
                return line if path == mountinfo else real_read_text(path, *args, **kwargs)
            with self.subTest(mount=suffix), patch.object(Path, "exists", exists), \
                    patch.object(Path, "read_text", read_text), patch.object(storage.os.path, "ismount", return_value=False):
                self.refused("mounted content")

    def test_prepare_overrides_a_restrictive_umask(self):
        self.cleanup()
        previous = os.umask(0o777)
        try:
            prepared = storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
        finally:
            os.umask(previous)
        self.assertEqual(stat.S_IMODE(prepared.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((prepared / storage.MARKER).stat().st_mode), 0o600)


if __name__ == "__main__":
    unittest.main()
