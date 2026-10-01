#!/usr/bin/env python3
"""Disposable filesystem regressions; never opens the host admission locks."""
import fcntl
import importlib.util
import os
from pathlib import Path
import stat
import tempfile
import unittest
import shutil
import subprocess
import sys
sys.dont_write_bytecode = True

source = Path(__file__).resolve().parents[1] / "deploy/local-linux/provision-docker-admission-locks.py"
spec = importlib.util.spec_from_file_location("admission_provision", source)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AdmissionProvisionTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="chariox-lock-test-")
        self.root = Path(self.scratch.name)
        self.root.chmod(0o755)
        (self.root / "tmp").mkdir(mode=0o1777)
        (self.root / "tmp").chmod(0o1777)
        self.lock = self.root / "tmp" / module.NAMES[0]

    def tearDown(self):
        self.scratch.cleanup()

    def test_preserves_legacy_inode_and_active_lock_when_provisioned_twice(self):
        self.lock.touch(mode=0o600)
        legacy = os.open(self.lock, os.O_RDWR)
        try:
            fcntl.flock(legacy, fcntl.LOCK_EX)
            inode = os.fstat(legacy).st_ino
            module.provision(self.root)
            module.provision(self.root)
            self.assertEqual(self.lock.stat().st_ino, inode)
            self.assertEqual(stat.S_IMODE(self.lock.stat().st_mode), 0o444)
            current = os.open(self.lock, os.O_RDONLY)
            try:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(current, fcntl.LOCK_EX | fcntl.LOCK_NB)
                fcntl.flock(legacy, fcntl.LOCK_UN)
                fcntl.flock(current, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                os.close(current)
        finally:
            os.close(legacy)

    def test_refuses_symlink_hardlink_writable_or_nonempty_legacy_lock(self):
        target = self.root / "target"
        target.touch()
        self.lock.symlink_to(target)
        with self.assertRaises(OSError):
            module.provision(self.root)
        self.lock.unlink()
        self.lock.touch(mode=0o600)
        alias = self.root / "alias"
        os.link(self.lock, alias)
        with self.assertRaises(PermissionError):
            module.provision(self.root)
        alias.unlink()
        self.lock.chmod(0o666)
        with self.assertRaises(PermissionError):
            module.provision(self.root)
        self.lock.chmod(0o600)
        self.lock.write_bytes(b"unexpected")
        with self.assertRaises(PermissionError):
            module.provision(self.root)

    def test_dry_run_does_not_create_or_change_locks(self):
        module.provision(self.root, dry_run=True)
        self.assertFalse(self.lock.exists())
        self.lock.touch(mode=0o600)
        inode = self.lock.stat().st_ino
        module.provision(self.root, dry_run=True)
        self.assertEqual(self.lock.stat().st_ino, inode)
        self.assertEqual(stat.S_IMODE(self.lock.stat().st_mode), 0o600)

    @unittest.skipUnless(os.geteuid() == 0, "staged root install probe requires root runner")
    def test_staged_install_never_calls_host_systemd_and_boot_recreates_reset_tmp(self):
        release = self.root / "release"
        context = release / "usr/lib/chariox/slice-build-context"
        script_target = context / "deploy/local-linux/provision-docker-admission-locks.py"
        unit_target = context / "deploy/managed-kernel/chariox-docker-admission-locks.service"
        script_target.parent.mkdir(parents=True)
        unit_target.parent.mkdir(parents=True)
        shutil.copyfile(source, script_target)
        unit_source = source.parents[1] / "managed-kernel/chariox-docker-admission-locks.service"
        shutil.copyfile(unit_source, unit_target)
        commands = self.root / "bin"
        commands.mkdir()
        forbidden = commands / "systemctl"
        forbidden.write_text("#!/bin/sh\nexit 83\n")
        forbidden.chmod(0o755)
        helper = source.parents[1] / "managed-kernel/docker-admission-install.sh"
        env = dict(os.environ, PATH=f"{commands}:{os.environ['PATH']}")
        subprocess.run(["sh", "-c", '. "$1"; install_docker_admission_artifacts "$2" "$3"',
                        "probe", str(helper), str(release), str(self.root)], env=env, check=True)
        self.assertTrue(self.lock.exists())
        installed = self.root / "usr/libexec/chariox-docker-admission-locks"
        self.assertEqual(installed.read_bytes(), source.read_bytes())
        unit = (self.root / "etc/systemd/system/chariox-docker-admission-locks.service").read_text()
        self.assertIn("Before=basic.target chariox-managed-bootstrap.service", unit)
        self.assertIn("ExecStart=/usr/bin/python3 /usr/libexec/chariox-docker-admission-locks", unit)
        # Model a volatile /tmp reset with no active test holder, then execute
        # the actual installed boot program against the staged root.
        for name in module.NAMES:
            (self.root / "tmp" / name).unlink()
        subprocess.run(["python3", str(installed), "--root", str(self.root)], check=True)
        for name in module.NAMES:
            self.assertEqual(stat.S_IMODE((self.root / "tmp" / name).stat().st_mode), 0o444)

    @unittest.skipUnless(os.geteuid() == 0, "cross-UID proof requires disposable root runner")
    def test_ordinary_uid_contends_with_legacy_root_and_cannot_replace_inode(self):
        self.lock.touch(mode=0o600)
        legacy = os.open(self.lock, os.O_RDWR)
        fcntl.flock(legacy, fcntl.LOCK_EX)
        module.provision(self.root)
        from_child, to_parent = os.pipe()
        from_parent, to_child = os.pipe()
        pid = os.fork()
        if pid == 0:
            try:
                os.close(from_child); os.close(to_child); os.close(legacy)
                os.setgroups([]); os.setgid(65534); os.setuid(65534)
                fd = os.open(self.lock, os.O_RDONLY)
                self.assertEqual(os.fstat(fd).st_ino, self.lock.stat().st_ino)
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with self.assertRaises(PermissionError):
                    self.lock.unlink()
                os.write(to_parent, b"blocked")
                os.read(from_parent, 1)
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                os.write(to_parent, b"acquired")
                os.close(fd)
                os._exit(0)
            except BaseException:
                os._exit(1)
        os.close(to_parent); os.close(from_parent)
        try:
            self.assertEqual(os.read(from_child, 7), b"blocked")
            fcntl.flock(legacy, fcntl.LOCK_UN)
            os.write(to_child, b"x")
            self.assertEqual(os.read(from_child, 8), b"acquired")
            self.assertEqual(os.waitpid(pid, 0)[1], 0)
        finally:
            os.close(legacy); os.close(from_child); os.close(to_child)


if __name__ == "__main__":
    unittest.main()
