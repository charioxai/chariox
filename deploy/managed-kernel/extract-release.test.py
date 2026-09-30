#!/usr/bin/env python3
import errno
import gzip
import importlib.util
import io
import os
import signal
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("extract_release", Path(__file__).with_name("extract-release.py"))
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


storage_spec = importlib.util.spec_from_file_location("release_update_storage", Path(__file__).with_name("release-update-storage.py"))
storage = importlib.util.module_from_spec(storage_spec)
storage_spec.loader.exec_module(storage)
UPDATE_ID = "managed_release_update_11111111-1111-1111-1111-111111111111"
OTHER_UPDATE_ID = "managed_release_update_22222222-2222-2222-2222-222222222222"


class ExtractionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="chariox-extraction-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.archive = self.root / "input.tar.gz"
        self.destination = self.root / "output"
        self.limits = module.Limits(file_bytes=4 * 1024**2, reserve_bytes=0, reserve_inodes=0)

    def write_archive(self, entries):
        with tarfile.open(self.archive, "w:gz", format=tarfile.PAX_FORMAT) as archive:
            for name, kind, value in entries:
                member = tarfile.TarInfo(name)
                member.mode = 0o755
                member.type = kind
                if kind == tarfile.REGTYPE:
                    member.size = len(value)
                    archive.addfile(member, io.BytesIO(value))
                else:
                    if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                        member.linkname = value
                    archive.addfile(member)

    def extract(self, limits=None):
        module.extract_release(self.archive, self.destination, limits or self.limits)

    def refused(self, pattern, limits=None):
        with self.assertRaisesRegex(module.UnsafeArchive, pattern):
            self.extract(limits)
        self.assertFalse(self.destination.exists())
        self.assertEqual(list(self.root.glob(".release-extract-*")), [])

    def test_valid_release_with_safe_links_and_long_names(self):
        long_name = "rootfs/" + "x" * 150
        self.write_archive([
            (".", tarfile.DIRTYPE, None), ("rootfs", tarfile.DIRTYPE, None),
            ("rootfs/bin", tarfile.DIRTYPE, None), (long_name, tarfile.REGTYPE, b"kernel"),
            ("rootfs/bin/kernel", tarfile.SYMTYPE, "../" + "x" * 150),
            ("rootfs/hard", tarfile.LNKTYPE, long_name),
        ])
        self.extract()
        self.assertEqual((self.destination / "rootfs/bin/kernel").read_bytes(), b"kernel")
        self.assertEqual((self.destination / "rootfs/hard").stat().st_ino, (self.destination / long_name).stat().st_ino)
        self.assertEqual(list(self.root.glob(".release-extract-*")), [])

    def test_sparse_archive_rejected_before_member_writes(self):
        source = self.root / "sparse"
        with source.open("wb") as file:
            file.seek(64 * 1024**2 - 1)
            file.write(b"x")
        subprocess.run(["tar", "--sparse", "-czf", str(self.archive), "-C", str(self.root), "sparse"], check=True)
        self.refused("sparse")

    def test_member_count_and_block_allocation_bounds(self):
        self.write_archive([(str(i), tarfile.REGTYPE, b"") for i in range(5)])
        self.refused("too many", module.Limits(members=4, reserve_bytes=0, reserve_inodes=0))
        self.refused("storage limit", module.Limits(file_bytes=4096, reserve_bytes=0, reserve_inodes=0))

    def test_absolute_traversal_and_duplicates_rejected(self):
        for name in ["../escape", "/tmp/escape", "rootfs/../escape"]:
            with self.subTest(name=name):
                self.write_archive([(name, tarfile.REGTYPE, b"x")])
                self.refused("escapes")
        self.write_archive([("file", tarfile.REGTYPE, b"a"), ("file", tarfile.REGTYPE, b"b")])
        self.refused("duplicate")

    def test_link_escape_link_parent_and_device_rejected(self):
        cases = [
            [("escape", tarfile.SYMTYPE, "/tmp")],
            [("escape", tarfile.SYMTYPE, "../elsewhere")],
            [("escape", tarfile.LNKTYPE, "../elsewhere")],
            [("link", tarfile.SYMTYPE, "dir"), ("dir", tarfile.DIRTYPE, None), ("link/file", tarfile.REGTYPE, b"x")],
            [("device", tarfile.CHRTYPE, None)],
        ]
        for entries in cases:
            with self.subTest(entries=entries):
                self.write_archive(entries)
                self.refused("symlink|escapes|parent|unsupported")

    def test_stream_limit_and_truncated_gzip_clean_scratch(self):
        self.write_archive([("file", tarfile.REGTYPE, b"x" * 100000)])
        self.refused("tar stream", module.Limits(tar_bytes=4096, reserve_bytes=0, reserve_inodes=0))
        self.archive.write_bytes(self.archive.read_bytes()[:-10])
        with self.assertRaises(EOFError):
            self.extract()
        self.assertFalse(self.destination.exists())
        self.assertEqual(list(self.root.glob(".release-extract-*")), [])

    def test_disk_and_inode_admission(self):
        self.write_archive([("file", tarfile.REGTYPE, b"x")])
        real = os.statvfs(self.root)
        values = list(real)
        values[4] = 0
        with patch.object(module.os, "statvfs", return_value=os.statvfs_result(values)):
            self.refused("disk headroom")
        values = list(real)
        values[7] = 0
        with patch.object(module.os, "statvfs", return_value=os.statvfs_result(values)):
            self.refused("inode headroom")

    def test_oversized_and_global_metadata_rejected(self):
        with tarfile.open(self.archive, "w:gz", format=tarfile.PAX_FORMAT) as archive:
            member = tarfile.TarInfo("file")
            member.pax_headers = {"comment": "x" * 70000}
            archive.addfile(member, io.BytesIO(b""))
        self.refused("oversized archive metadata")
        with tarfile.open(self.archive, "w:gz", format=tarfile.PAX_FORMAT, pax_headers={"comment": "global"}) as archive:
            archive.addfile(tarfile.TarInfo("file"), io.BytesIO(b""))
        self.refused("global archive metadata")

    def test_hardlink_copy_amplification_is_charged(self):
        self.write_archive([("large", tarfile.REGTYPE, b"x" * 65536)] +
                           [("copy" + str(i), tarfile.LNKTYPE, "large") for i in range(4)])
        self.refused("hardlinks exceed", module.Limits(file_bytes=131072, reserve_bytes=0, reserve_inodes=0))

    def test_directory_modes_preserved_under_restrictive_umask(self):
        with tarfile.open(self.archive, "w:gz") as archive:
            for name, mode in [("rootfs", 0o755), ("rootfs/private", 0o700)]:
                member = tarfile.TarInfo(name)
                member.type = tarfile.DIRTYPE
                member.mode = mode
                archive.addfile(member)
            member = tarfile.TarInfo("rootfs/private/file")
            member.mode = 0o644
            archive.addfile(member, io.BytesIO(b""))
        previous = os.umask(0o077)
        try:
            self.extract()
        finally:
            os.umask(previous)
        self.assertEqual((self.destination / "rootfs").stat().st_mode & 0o777, 0o755)
        self.assertEqual((self.destination / "rootfs/private").stat().st_mode & 0o777, 0o700)
        self.assertEqual((self.destination / "rootfs/private/file").stat().st_mode & 0o777, 0o644)

    def test_aggregate_pax_metadata_rejected_before_publication(self):
        with tarfile.open(self.archive, "w:gz", format=tarfile.PAX_FORMAT) as archive:
            for index in range(4):
                member = tarfile.TarInfo("file" + str(index))
                member.pax_headers = {"comment": "x" * 2000}
                archive.addfile(member, io.BytesIO(b""))
        self.refused("aggregate archive metadata", module.Limits(
            metadata_total_bytes=4096, reserve_bytes=0, reserve_inodes=0,
        ))

    def test_aggregate_gnu_long_metadata_rejected_before_publication(self):
        with tarfile.open(self.archive, "w:gz", format=tarfile.GNU_FORMAT) as archive:
            for index in range(4):
                archive.addfile(tarfile.TarInfo("x" * 150 + str(index)), io.BytesIO(b""))
        self.refused("aggregate archive metadata", module.Limits(
            metadata_total_bytes=2048, reserve_bytes=0, reserve_inodes=0,
        ))

    def test_publication_filesystem_requires_its_own_disk_and_inode_headroom(self):
        self.write_archive([("file", tarfile.REGTYPE, b"x")])
        publication = self.root / "publication"
        publication.mkdir()
        real_statvfs = module.os.statvfs
        real_stat = module.os.stat
        publication_stat = real_stat(publication)
        def separate_stat(path, *args, **kwargs):
            value = real_stat(path, *args, **kwargs)
            if Path(path) == publication:
                fields = list(value)
                fields[2] = publication_stat.st_dev + 1
                return os.stat_result(fields)
            return value
        for exhausted, pattern in [(4, "disk headroom"), (7, "inode headroom")]:
            def capacity(path):
                value = real_statvfs(path)
                if Path(path) == publication:
                    fields = list(value)
                    fields[exhausted] = 0
                    return os.statvfs_result(fields)
                return value
            destination = self.root / ("output-" + str(exhausted))
            with self.subTest(resource=pattern), patch.object(module.os, "stat", side_effect=separate_stat), \
                    patch.object(module.os, "statvfs", side_effect=capacity):
                with self.assertRaisesRegex(module.UnsafeArchive, pattern):
                    module.extract_release(self.archive, destination, self.limits, publication)
                self.assertFalse(destination.exists())
                self.assertEqual(list(self.root.glob(".release-extract-*")), [])

    @unittest.skipUnless(os.name == "posix", "requires POSIX process interruption and FIFO")
    def test_interrupted_extraction_restarts_without_leaving_or_deleting_other_attempts(self):
        staging_root = self.root / "update-staging"
        other = storage.prepare(staging_root, OTHER_UPDATE_ID, expected_uid=os.geteuid())
        (other / "keep").write_text("other attempt")
        unrelated = staging_root / ".release-extract-unowned"
        unrelated.mkdir()
        (unrelated / "keep").write_text("unowned sibling")
        # Keep the neighbor and all of its durable ownership records intact.
        # This snapshot also detects any scratch or journal leaked by our attempt.
        survivors = set(staging_root.iterdir())
        survivor_records = {path: path.read_bytes() for path in survivors if path.is_file()}
        restart_cleanup = (
            "import importlib.util,os,sys; "
            "spec=importlib.util.spec_from_file_location('storage',sys.argv[1]); "
            "module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module); "
            "module.cleanup(sys.argv[2],sys.argv[3],expected_uid=os.geteuid())"
        )
        for termination in [signal.SIGTERM, signal.SIGKILL]:
            with self.subTest(signal=termination):
                attempt = storage.prepare(staging_root, UPDATE_ID, expected_uid=os.geteuid())
                destination = attempt / "extracted"
                fifo = self.root / ("archive-fifo-" + str(termination))
                os.mkfifo(fifo, 0o600)
                process = subprocess.Popen([
                    sys.executable, str(Path(__file__).with_name("extract-release.py")),
                    str(fifo), str(destination),
                ], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                writer = None
                try:
                    deadline = time.monotonic() + 5
                    while writer is None and time.monotonic() < deadline:
                        try:
                            writer = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
                        except OSError as error:
                            if error.errno != errno.ENXIO:
                                raise
                            time.sleep(0.01)
                    self.assertIsNotNone(writer, "extractor did not open fixture archive")
                    # Valid compressed data with its trailer withheld leaves the
                    # extractor waiting after it has written megabytes of tar.
                    compressed = gzip.compress(os.urandom(3 * 1024**2), mtime=0)[:-8]
                    written = 0
                    while written < len(compressed) and time.monotonic() < deadline:
                        try:
                            written += os.write(writer, memoryview(compressed)[written:])
                        except BlockingIOError:
                            time.sleep(0.005)
                    self.assertEqual(written, len(compressed), "fixture archive writer stalled")
                    scratch = []
                    while time.monotonic() < deadline:
                        scratch = list(attempt.glob(".release-extract-*/archive.tar"))
                        if scratch and scratch[0].stat().st_size >= 1024**2:
                            break
                        self.assertIsNone(process.poll(), "extractor exited before fixture interruption")
                        time.sleep(0.01)
                    self.assertTrue(scratch and scratch[0].stat().st_size >= 1024**2,
                                    "fixture did not produce persistent extraction scratch")
                    self.assertEqual(scratch[0].parents[1], attempt)
                    self.assertEqual(list(staging_root.glob(".release-extract-*")), [unrelated])
                    process.send_signal(termination)
                    self.assertEqual(process.wait(timeout=3), -termination)
                    self.assertTrue(scratch[0].exists(), "fixture must interrupt before Python cleanup")
                finally:
                    if process.poll() is None:
                        process.kill()
                    process.communicate(timeout=3)
                    if writer is not None:
                        os.close(writer)
                    fifo.unlink()
                # A fresh process discovers the persisted ownership marker,
                # removes the exact attempt, and leaves both siblings intact.
                subprocess.run([
                    sys.executable, "-c", restart_cleanup,
                    str(Path(__file__).with_name("release-update-storage.py")),
                    str(staging_root), UPDATE_ID,
                ], check=True, capture_output=True, text=True, timeout=5)
                self.assertFalse(attempt.exists())
                self.assertEqual((other / "keep").read_text(), "other attempt")
                self.assertEqual((unrelated / "keep").read_text(), "unowned sibling")
                self.assertEqual(set(staging_root.iterdir()), survivors)
                for path, contents in survivor_records.items():
                    self.assertEqual(path.read_bytes(), contents)
                # The same identity can be retried without remembering any
                # random scratch name from the interrupted extractor process.
                retry = storage.prepare(staging_root, UPDATE_ID, expected_uid=os.geteuid())
                self.write_archive([("file", tarfile.REGTYPE, b"retried release")])
                module.extract_release(self.archive, retry / "extracted", self.limits)
                self.assertEqual((retry / "extracted/file").read_bytes(), b"retried release")
                self.assertEqual(list(retry.glob(".release-extract-*")), [])
                storage.cleanup(staging_root, UPDATE_ID, expected_uid=os.geteuid())

    def test_existing_destination_never_modified(self):
        self.write_archive([("file", tarfile.REGTYPE, b"x")])
        self.destination.mkdir()
        marker = self.destination / "keep"
        marker.write_text("keep")
        with self.assertRaisesRegex(module.UnsafeArchive, "must be absent"):
            self.extract()
        self.assertEqual(marker.read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
