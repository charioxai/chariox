"""Regression audit for stopped-container retirement proofs, no live processes."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import uuid

SPEC = importlib.util.spec_from_file_location('marker_store', Path(__file__).with_name('browser-upload-store.py'))
store = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(store)


class RetirementMarkerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='upload-marker-audit-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve() / 'uploads'
        self.root.mkdir(mode=0o700)
        environment = patch.dict(os.environ, {'CHARIOX_BROWSER_LIFECYCLE_ROOT': str(Path(temporary.name).resolve() / 'lifetimes')})
        environment.start()
        self.addCleanup(environment.stop)
        self.owner = {'pid': 100, 'start': '12345', 'group': 100, 'session': 100,
                      'uid': os.getuid(), 'boot': 'old-boot', 'namespace': 'pid:[123]'}
        self.record = {'version': 1, 'instance': uuid.uuid4().hex, 'profile': '/tmp/audit-profile',
                       'supervisor': self.owner, 'browser': None}
        self.proof = {'version': 1, 'authority': 'docker-stopped-container', 'engineId': 'engine-1234',
                      'containerId': 'a' * 64, 'startedAt': '2026-09-27T19:00:00Z', 'finishedAt': '2026-09-27T19:01:00Z'}

    def entry(self, phase='preparing', lifetime=True):
        entry = {'id': str(uuid.uuid4()), 'bytes': 8, 'count': 1, 'phase': phase,
                 'owner': self.owner, 'lifetime': self.record if lifetime else None}
        (self.root / entry['id']).mkdir()
        (self.root / entry['id'] / 'selected-file').write_bytes(b'live-data')
        store.lifecycle.write_json(self.root / 'ledger.json', [entry])
        return entry

    def browser_retirement(self):
        root = store.lifecycle.directory()
        store.lifecycle.write_json(root / f"{self.record['instance']}.retired.json", self.record)
        store.lifecycle.write_json(store.lifecycle.current_path(root, self.record['profile']), self.record)
        return root / f"{self.record['instance']}.container-retired.json"

    def test_invalid_dates_and_reversed_container_intervals_do_not_authorize_retirement(self):
        for started, finished in [('2026-99-99T99::Z', '2026-99-99T99::Z'),
                                  ('2026-09-27T19:01:00Z', '2026-09-27T19:00:00Z')]:
            with self.subTest(started=started):
                self.assertFalse(store.valid_container_boundary({**self.proof, 'startedAt': started, 'finishedAt': finished}))

    def test_malformed_companion_cannot_reclaim_live_preparing_files(self):
        entry = self.entry()
        marker = self.browser_retirement()
        store.lifecycle.write_json(marker, {**self.proof, 'startedAt': '2026-99-99T99::Z',
                                           'finishedAt': '2026-99-99T99::Z', 'lifetime': self.record})
        with patch.object(store, 'process_state', return_value='alive'):
            try:
                store.transact(self.root, {'action': 'reap'})
            except RuntimeError:
                pass
        self.assertTrue((self.root / entry['id'] / 'selected-file').exists(), 'malformed stopped-container marker reclaimed a live preparation')

    def test_malformed_legacy_marker_cannot_reclaim_exposed_files(self):
        entry = self.entry(phase='exposed', lifetime=False)
        store.lifecycle.write_json(self.root / 'legacy-container-retired.json',
                                   {**self.proof, 'finishedAt': '2026-09-27T18:59:00Z', 'entries': [entry]})
        try:
            store.transact(self.root, {'action': 'reap'})
        except RuntimeError:
            pass
        self.assertTrue((self.root / entry['id'] / 'selected-file').exists(), 'reversed stopped-container marker reclaimed exposed files')

    def test_browser_only_retirement_preserves_live_preparing_files(self):
        entry = self.entry()
        self.browser_retirement()
        with patch.object(store, 'process_state', return_value='alive'):
            store.transact(self.root, {'action': 'reap'})
        self.assertTrue((self.root / entry['id'] / 'selected-file').exists())

    def test_stale_legacy_generation_does_not_match_reused_entry_id(self):
        entry = self.entry(phase='exposed', lifetime=False)
        stale = {**entry, 'owner': {**self.owner, 'start': 'previous-process'}}
        store.lifecycle.write_json(self.root / 'legacy-container-retired.json', {**self.proof, 'entries': [stale]})
        store.transact(self.root, {'action': 'reap'})
        self.assertTrue((self.root / entry['id'] / 'selected-file').exists())

    def test_marker_without_ledger_refuses_without_touching_files(self):
        entry = self.entry(phase='exposed', lifetime=False)
        (self.root / 'ledger.json').unlink()
        store.lifecycle.write_json(self.root / 'legacy-container-retired.json', {**self.proof, 'entries': [entry]})
        with self.assertRaisesRegex(RuntimeError, 'ledger is missing'):
            store.transact(self.root, {'action': 'reap'})
        self.assertTrue((self.root / entry['id'] / 'selected-file').exists())

    def test_pruning_removes_companion_only_after_both_ledger_and_pointer_release(self):
        entry = self.entry(phase='exposed')
        marker = self.browser_retirement()
        store.lifecycle.write_json(marker, {**self.proof, 'lifetime': self.record})
        directory = store.lifecycle.directory()
        store.prune_receipts([entry])
        self.assertTrue(marker.exists())
        store.prune_receipts([])
        self.assertTrue(marker.exists())
        store.lifecycle.write_json(store.lifecycle.current_path(directory, self.record['profile']),
                                   {**self.record, 'instance': uuid.uuid4().hex})
        store.prune_receipts([])
        self.assertFalse(marker.exists())


if __name__ == '__main__':
    unittest.main()
