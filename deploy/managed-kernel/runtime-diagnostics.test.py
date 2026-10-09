#!/usr/bin/env python3
# MP-07/MP-08/MP-10/MP-11: diagnostic confidentiality, durability and ack failures.
import importlib.util,json,os,pathlib,tempfile,unittest
spec=importlib.util.spec_from_file_location('diagnostics', pathlib.Path(__file__).with_name('runtime-diagnostics.py'))
diag=importlib.util.module_from_spec(spec);spec.loader.exec_module(diag)
class Diagnostics(unittest.TestCase):
    def test_mp11_schema_rejects_payloads_and_unsafe_pids(self):
        valid={'schema':1,'atMs':1,'pid':20,'event':'prompt_dispatch'}
        for value in ({**valid,'prompt':'SECRET'}, {**valid,'event':'SECRET'}, {**valid,'pid':1}, {**valid,'pid':True}, {**valid,'schema':True}):
            with self.assertRaises(ValueError):diag.canonical(value)
    def test_mp07_ship_ack_is_exact_and_replay_is_idempotent(self):
        with tempfile.TemporaryDirectory() as a,tempfile.TemporaryDirectory() as b:
            guest,observer=pathlib.Path(a),pathlib.Path(b)
            for event in ('prompt_dispatch','heartbeat_sent','activated'):diag.event(guest,event)
            seen=set()
            post=lambda raw:{'sha256':diag.accept(observer,raw)}
            first=diag.ship(guest,'https://observer.example/path1-diagnostics/round-20261009',seen,post)
            self.assertEqual(first['records'],3);self.assertEqual(len(list(observer.glob('*.json'))),3)
            again=diag.ship(guest,'https://observer.example/path1-diagnostics/round-20261009',seen,lambda raw:self.fail('duplicate sent'))
            self.assertEqual(first['snapshotSha256'],again['snapshotSha256'])
            with self.assertRaises(ValueError):diag.ship(guest,'https://observer.example/',set(),lambda raw:{'sha256':'wrong'})
            with self.assertRaises(ValueError):diag.ship(guest,'http://observer.example/',set(),post)
            with self.assertRaises(ValueError):diag.ship(guest,'https://secret@observer.example/',set(),post)
    def test_mp11_links_permissions_and_partial_record_fail_closed(self):
        with tempfile.TemporaryDirectory() as a,tempfile.TemporaryDirectory() as b:
            guest=pathlib.Path(a);outside=pathlib.Path(b)/'outside';outside.write_text('SECRET')
            link=guest/'runtime-20-1.jsonl';link.symlink_to(outside)
            with self.assertRaises(OSError):list(diag.records(guest))
            link.unlink();os.link(outside,link)
            with self.assertRaises(ValueError):list(diag.records(guest))
            link.unlink();link.write_text('{"schema":1');link.chmod(0o600)
            with self.assertRaises(ValueError):list(diag.records(guest))
            guest.chmod(0o755)
            with self.assertRaises(ValueError):diag.event(guest,'heartbeat_sent')
    def test_mp07_observer_fsync_before_ack(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as a:
            raw=diag.canonical({'schema':1,'atMs':1,'pid':20,'event':'update_applied'})
            with patch.object(diag.os,'fsync',side_effect=OSError('disk')):
                with self.assertRaises(OSError):diag.accept(pathlib.Path(a),raw)
            diag.accept(pathlib.Path(a),raw)
    def test_mp11_installer_rejects_existing_and_dangling_configuration(self):
        import subprocess
        source=pathlib.Path(__file__).with_name('enable-campaign-diagnostics.sh').read_text()
        guard=source.split('dropin=/etc/systemd/system/chariox-path1-managed-bootstrap.service.d\n',1)[1].split('install -d -m 0700',1)[0]
        with tempfile.TemporaryDirectory() as a:
            root=pathlib.Path(a);config=root/'path1-campaign-diagnostics.conf'
            guard=guard.replace('/etc/systemd/system/chariox-path1-campaign-diagnostics.service',str(root/'campaign.service'))
            script='set -eu; dropin=$1\n'+guard+'\nprintf allowed\n'
            config.write_text('retained configuration')
            result=subprocess.run(['/bin/sh','-c',script,'guard',a],capture_output=True)
            self.assertEqual(result.returncode,1);self.assertEqual(config.read_text(),'retained configuration')
            config.unlink();config.symlink_to(root/'missing')
            result=subprocess.run(['/bin/sh','-c',script,'guard',a],capture_output=True)
            self.assertEqual(result.returncode,1)
            config.unlink()
            self.assertEqual(subprocess.run(['/bin/sh','-c',script,'guard',a],capture_output=True).returncode,0)
    def test_mp07_phase_diagnostic_never_masks_failed_durable_write(self):
        import subprocess
        source=pathlib.Path(__file__).with_name('upgrade-image.sh').read_text()
        function=source.split('write_phase() {',1)[1].split('\n}',1)[0]
        script='node() { return 23; }; record_diagnostic_phase() { return 0; }; write_phase() {'+function+'\n}; write_phase stopped'
        result=subprocess.run(['/bin/sh','-c',script],capture_output=True)
        self.assertEqual(result.returncode,23)
    def test_mp07_upgrade_wires_only_real_phase_after_commit(self):
        source=pathlib.Path(__file__).with_name('upgrade-image.sh').read_text()
        self.assertIn('atomic-text "$1" "$transaction_root/phase" || return $?\n  record_diagnostic_phase "$1"',source)
        self.assertIn('transaction_active=1\nrecord_diagnostic_phase prepared\n\nif ! systemctl stop',source)
if __name__=='__main__':unittest.main()
