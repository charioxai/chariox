#!/usr/bin/env python3
"""Exact-attempt cleanup refuses uncertain ownership and shared storage."""
import importlib.util
import os
import json
from pathlib import Path
import stat
import signal
import subprocess
import sys
import tempfile
import time
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

    def interrupted_operation(self, operation, after, *, error=False, root=None):
        # Run the real helper and filesystem operation, then abruptly interrupt
        # its process at the ownership/deletion boundary being exercised.
        script = """
import importlib.util, os, signal, sys
from pathlib import Path
spec = importlib.util.spec_from_file_location('storage', sys.argv[1])
storage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(storage)
root = Path(sys.argv[2])
update_id, operation, after, error = sys.argv[3:]
path = root / update_id

def stop():
    if error == 'True':
        raise OSError('injected filesystem operation error')
    os.kill(os.getpid(), signal.SIGKILL)

if after in ('mkdir', 'root-mkdir'):
    real_mkdir = Path.mkdir
    def mkdir(self, *args, **kwargs):
        result = real_mkdir(self, *args, **kwargs)
        if self == (root if after == 'root-mkdir' else path):
            stop()
        return result
    Path.mkdir = mkdir
elif after == 'marker-unlink':
    real_unlink = os.unlink
    def unlink(name, *args, **kwargs):
        result = real_unlink(name, *args, **kwargs)
        if os.fspath(name) == storage.MARKER or Path(name) == path / storage.MARKER:
            stop()
        return result
    os.unlink = unlink
elif after in ('journal-write', 'marker-write'):
    real_write = os.write
    def write(fd, data):
        target = os.readlink('/proc/self/fd/' + str(fd))
        suffix = '.owner.tmp' if after == 'journal-write' else '.marker.tmp'
        if path.exists() and target.endswith(suffix):
            real_write(fd, data[:5])
            stop()
        return real_write(fd, data)
    os.write = write
elif after == 'boundary-rmdir':
    real_rmdir = os.rmdir
    def rmdir(name, *args, **kwargs):
        result = real_rmdir(name, *args, **kwargs)
        if Path(name) == path:
            stop()
        return result
    os.rmdir = rmdir
getattr(storage, operation)(root, update_id, expected_uid=os.geteuid())
"""
        result = subprocess.run([
            sys.executable, '-B', '-c', script, str(Path(storage.__file__)),
            str(root or self.root), UPDATE_ID, operation, after, str(error),
        ], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 1 if error else -signal.SIGKILL, result.stderr)

    def test_prepare_recovers_after_process_is_killed_immediately_after_mkdir(self):
        self.cleanup()
        self.interrupted_operation('prepare', 'mkdir')
        self.assertTrue(self.attempt.exists())
        self.assertFalse((self.attempt / storage.MARKER).exists())
        prepared = storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
        self.assertEqual(prepared, self.attempt)
        self.cleanup()
        self.assertFalse(self.attempt.exists())

    def test_cleanup_recovers_after_process_is_killed_after_internal_marker_unlink(self):
        other = storage.prepare(self.root, OTHER_UPDATE_ID, expected_uid=self.uid)
        (other / 'keep').write_text('other attempt')
        unknown = self.root / 'unknown-sibling'
        unknown.mkdir()
        (unknown / 'keep').write_text('unknown scratch')
        self.interrupted_operation('cleanup', 'marker-unlink')
        self.assertTrue(self.attempt.exists())
        self.assertFalse((self.attempt / storage.MARKER).exists())
        self.cleanup()
        self.cleanup()
        self.assertFalse(self.attempt.exists())
        self.assertEqual((other / 'keep').read_text(), 'other attempt')
        self.assertEqual((unknown / 'keep').read_text(), 'unknown scratch')

    def test_filesystem_errors_do_not_leave_an_unrecoverable_attempt(self):
        for operation, after in [('prepare', 'mkdir'), ('cleanup', 'marker-unlink')]:
            with self.subTest(operation=operation):
                self.cleanup()
                if operation == 'cleanup':
                    storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
                    self.keep.write_text('this attempt')
                self.interrupted_operation(operation, after, error=True)
                prepared = storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
                self.assertEqual(prepared, self.attempt)
                self.cleanup()
                self.assertFalse(self.attempt.exists())

    def test_torn_unpublished_journal_or_marker_write_recovers_from_published_proof(self):
        for after in ['journal-write', 'marker-write']:
            with self.subTest(after=after):
                self.cleanup()
                self.interrupted_operation('prepare', after)
                self.assertTrue(self.attempt.exists())
                storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
                self.cleanup()
                self.assertFalse(self.attempt.exists())
                self.assertEqual(set(item.name for item in self.root.iterdir()), {storage.LOCK})

    def test_absent_boundary_retains_proof_until_parent_sync_and_retry(self):
        self.interrupted_operation('cleanup', 'boundary-rmdir')
        self.assertFalse(self.attempt.exists())
        self.assertTrue(storage.journal_path(self.attempt).exists())
        self.cleanup()
        self.cleanup()
        self.assertFalse(storage.journal_path(self.attempt).exists())

    def test_legacy_boundary_publishes_external_proof_before_interrupted_deletion(self):
        storage.journal_path(self.attempt).unlink()
        self.interrupted_operation('cleanup', 'marker-unlink')
        self.assertFalse((self.attempt / storage.MARKER).exists())
        self.cleanup()
        self.assertFalse(self.attempt.exists())

    def test_journal_rejects_root_and_boundary_inode_mismatch(self):
        journal = storage.journal_path(self.attempt)
        original = journal.read_text()
        for field, replacement in [('root', str(self.root) + '-different'), ('rootInode', 0), ('rootDevice', -1)]:
            with self.subTest(field=field):
                value = json.loads(original)
                value[field] = replacement
                journal.write_text(json.dumps(value))
                self.refused('journal.*root mismatch')
        journal.write_text(original)
        preserved = self.root / 'preserved-original-boundary'
        self.attempt.rename(preserved)
        self.attempt.mkdir(mode=0o700)
        with self.assertRaisesRegex(ValueError, 'inode mismatch'):
            self.cleanup()
        self.assertEqual((preserved / 'keep').read_text(), 'this attempt')
        self.assertTrue(self.attempt.exists())

    def test_creating_intent_refuses_nonempty_unknown_content(self):
        self.cleanup()
        self.interrupted_operation('prepare', 'mkdir')
        self.keep.write_text('unknown content')
        with self.assertRaisesRegex(ValueError, 'not empty'):
            self.cleanup()
        self.assertEqual(self.keep.read_text(), 'unknown content')

    def test_stale_unpublished_temporary_does_not_authorize_an_unknown_boundary(self):
        self.cleanup()
        self.attempt.mkdir(mode=0o700)
        self.keep.write_text('unknown content')
        temporary = storage.journal_path(self.attempt).with_name(storage.journal_path(self.attempt).name + '.tmp')
        temporary.write_text('{"version":')
        temporary.chmod(0o600)
        with self.assertRaises(OSError):
            storage.prepare(self.root, UPDATE_ID, expected_uid=self.uid)
        self.assertEqual(self.keep.read_text(), 'unknown content')
        self.assertFalse(storage.journal_path(self.attempt).exists())
        self.assertEqual(temporary.read_text(), '{"version":')

    def test_private_modes_survive_interruption_with_a_restrictive_umask(self):
        root = Path(self.temp.name).resolve() / 'new-root'
        previous = os.umask(0o777)
        try:
            self.interrupted_operation('prepare', 'root-mkdir', root=root)
            self.assertEqual(stat.S_IMODE(root.stat().st_mode), 0o700)
            self.interrupted_operation('prepare', 'mkdir', root=root)
            attempt = root / UPDATE_ID
            self.assertEqual(stat.S_IMODE(attempt.stat().st_mode), 0o700)
            storage.prepare(root, UPDATE_ID, expected_uid=self.uid)
            storage.cleanup(root, UPDATE_ID, expected_uid=self.uid)
        finally:
            os.umask(previous)
        self.assertFalse(attempt.exists())

    def test_cleanup_waits_for_preparation_ownership_publication(self):
        self.cleanup()
        script = """
import importlib.util, os, sys
from pathlib import Path
spec = importlib.util.spec_from_file_location('storage', sys.argv[1])
storage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(storage)
root, update_id, operation = Path(sys.argv[2]), sys.argv[3], sys.argv[4]
if operation == 'prepare':
    real_mkdir = Path.mkdir
    def mkdir(path, *args, **kwargs):
        result = real_mkdir(path, *args, **kwargs)
        if path == root / update_id:
            print('boundary-created', flush=True)
            sys.stdin.readline()
        return result
    Path.mkdir = mkdir
else:
    print('cleanup-started', flush=True)
getattr(storage, operation)(root, update_id, expected_uid=os.geteuid())
"""
        args = [sys.executable, '-B', '-c', script, str(Path(storage.__file__)), str(self.root), UPDATE_ID]
        first = subprocess.Popen(args + ['prepare'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        second = None
        try:
            self.assertEqual(first.stdout.readline().strip(), 'boundary-created')
            second = subprocess.Popen(args + ['cleanup'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            self.assertEqual(second.stdout.readline().strip(), 'cleanup-started')
            time.sleep(0.1)
            self.assertIsNone(second.poll(), 'cleanup raced ahead of ownership publication')
            first.stdin.write('continue\n')
            first.stdin.flush()
            _, first_error = first.communicate(timeout=10)
            _, second_error = second.communicate(timeout=10)
            self.assertEqual(first.returncode, 0, first_error)
            self.assertEqual(second.returncode, 0, second_error)
            self.assertFalse(self.attempt.exists())
        finally:
            for child in [first, second]:
                if child is not None and child.poll() is None:
                    child.kill()
                    child.communicate(timeout=10)

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
        storage.journal_path(self.attempt).unlink()
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
