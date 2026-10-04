"""MP-03/MP-10/MP-11: operator fixture refuses unsafe ownership and mutation."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('fixture', Path(__file__).resolve().parents[1] / 'deploy/local-linux/storage-qualification-fixture.py')
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)

@unittest.skipUnless(os.geteuid() == 0, 'isolated root fixtures required')
class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix='chariox-b201-storage-', dir='/root')
        self.root = Path(self.scratch.name)
        generation = self.root / 'backups/backup-1/generation-123456'
        generation.mkdir(parents=True, mode=0o700)
        self.archive = generation / 'home.tar.zst'
        self.archive.write_bytes(b'public synthetic archive')
        self.archive.chmod(0o600)
        self.manifest = self.root / 'manifest.json'
        self.state = dict(id='backup-1', source_slice_id='slice-1', home_archive_path=str(self.archive), manifest_path=str(self.manifest), size_bytes=self.archive.stat().st_size, image_ref='owned-fixture', name='corrupt-candidate')
        self.manifest.write_text(json.dumps(self.state))
        self.manifest.chmod(0o600)
        import hashlib
        metadata = generation / 'metadata.json'
        metadata.write_text(json.dumps(dict(schemaVersion=1,scope='backup',id='backup-1',sizeBytes=self.state['size_bytes'],sha256=hashlib.sha256(self.archive.read_bytes()).hexdigest())))
        metadata.chmod(0o600)
    def tearDown(self): self.scratch.cleanup()
    def apply(self, op='verify'):
        return fixture.apply(op, self.state, self.root, tar=lambda *a, **kw: None)
    def test_verify_and_corrupt_exact_candidate(self):
        self.assertTrue(self.apply()['private'])
        self.assertTrue(self.apply('corrupt')['corrupted'])
        self.assertEqual(self.archive.read_bytes(), b'deliberately corrupted backup archive')
    def test_known_good_backup_cannot_be_corrupted(self):
        self.state['name']='browser-baseline'
        with self.assertRaises(ValueError): self.apply('corrupt')
        self.assertEqual(self.archive.stat().st_size, 24)
    def test_manifest_mismatch_cannot_be_corrupted(self):
        self.state['id']='foreign-backup'
        with self.assertRaises(ValueError): self.apply('corrupt')
    def test_symlink_archive_is_refused(self):
        self.archive.rename(self.root / 'keep')
        self.archive.symlink_to(self.root / 'keep')
        with self.assertRaises(OSError): self.apply()
    def test_foreign_or_writable_ancestry_is_refused(self):
        self.root.chmod(0o777)
        with self.assertRaises(ValueError): self.apply()
        self.root.chmod(0o700)
        os.chown(self.root,65534,65534)
        with self.assertRaises(ValueError): self.apply()
    def test_hardlinked_archive_is_refused(self):
        os.link(self.archive, self.root / 'keep')
        with self.assertRaises(ValueError): self.apply()
    def test_escape_is_refused(self):
        with self.assertRaises(ValueError): fixture.pinned_open(self.manifest, self.root / 'other')
    def test_user_manifest_and_root_archive_are_separate_namespaces(self):
        user_root = self.root / 'ordinary-manifests'
        user_root.mkdir(mode=0o700)
        manifest = user_root / 'manifest.json'
        self.state['manifest_path'] = str(manifest)
        manifest.write_text(json.dumps(self.state))
        manifest.chmod(0o600)
        self.assertTrue(fixture.apply('verify', self.state, self.root / 'backups',
            manifest_root=user_root, tar=lambda *a, **kw: None)['private'])

if __name__ == '__main__': unittest.main()
