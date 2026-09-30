#!/usr/bin/env python3
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("extract_release", Path(__file__).with_name("extract-release.py"))
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class ExtractionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="chariox-extraction-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
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
