#!/usr/bin/env python3
"""MP-03/MP-10/MP-11: execute installer filesystem seams without Docker/host mutation."""
import ast
import os
from pathlib import Path
import stat
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'deploy/local-linux/install-local-docker-dev.py'
# Execute the real filesystem helpers alone, never privileged installation.
tree = ast.parse(SOURCE.read_text())
helpers = ast.Module(body=[node for node in tree.body if isinstance(node, (ast.Import, ast.ImportFrom))
                         or isinstance(node, ast.FunctionDef) and node.name in ('refuse', 'directory', 'publish')], type_ignores=[])
namespace = {}
exec(compile(helpers, str(SOURCE), 'exec'), namespace)

@unittest.skipUnless(os.geteuid() == 0, 'root ownership fixtures need an isolated root runner')
class InstallerFilesystemTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix='chariox-b201-install-', dir='/root')
        self.root = Path(self.scratch.name)
        self.parent = self.root / 'parent'
        self.parent.mkdir(mode=0o755)
        self.target = self.parent / 'control'

    def tearDown(self):
        self.scratch.cleanup()

    def test_normal_install_replay_retains_inode_and_bytes(self):
        namespace['directory'](self.target, 0o700)
        asset = self.target / 'public.json'
        namespace['publish'](asset, b'public fixture', 0o444)
        inode = asset.stat().st_ino
        namespace['directory'](self.target, 0o700)
        namespace['publish'](asset, b'public fixture', 0o444)
        self.assertEqual(asset.stat().st_ino, inode)
        self.assertEqual(asset.read_bytes(), b'public fixture')
        self.assertEqual(stat.S_IMODE(asset.stat().st_mode), 0o444)

    def test_foreign_ancestor_refused_before_creating_control_directory(self):
        os.chown(self.parent, 65534, 65534)
        with self.assertRaises(SystemExit):
            namespace['directory'](self.target, 0o700)
        self.assertFalse(self.target.exists())
        self.assertEqual(self.parent.stat().st_uid, 65534)

    def test_writable_ancestor_refused_before_publish(self):
        self.target.mkdir(mode=0o700)
        self.parent.chmod(0o777)
        asset = self.target / 'public.json'
        with self.assertRaises(SystemExit):
            namespace['publish'](asset, b'public fixture', 0o444)
        self.assertFalse(asset.exists())
        self.assertEqual(stat.S_IMODE(self.parent.stat().st_mode), 0o777)

    def test_foreign_existing_directory_refused_without_adoption(self):
        self.target.mkdir(mode=0o700)
        os.chown(self.target, 65534, 65534)
        inode = self.target.stat().st_ino
        with self.assertRaises(SystemExit):
            namespace['directory'](self.target, 0o700)
        self.assertEqual(self.target.stat().st_ino, inode)
        self.assertEqual(self.target.stat().st_uid, 65534)

    def test_symlink_ancestor_refused_without_outside_mutation(self):
        outside = self.root / 'outside'
        outside.mkdir()
        self.parent.rmdir()
        self.parent.symlink_to(outside, target_is_directory=True)
        with self.assertRaises(SystemExit):
            namespace['directory'](self.target, 0o700)
        self.assertEqual(list(outside.iterdir()), [])

    def test_incompatible_publication_preserves_old_inode_and_bytes(self):
        namespace['directory'](self.target, 0o700)
        asset = self.target / 'public.json'
        namespace['publish'](asset, b'prior fixture', 0o444)
        inode = asset.stat().st_ino
        with self.assertRaises(SystemExit):
            namespace['publish'](asset, b'changed fixture', 0o444)
        self.assertEqual(asset.stat().st_ino, inode)
        self.assertEqual(asset.read_bytes(), b'prior fixture')

if __name__ == '__main__':
    unittest.main()
