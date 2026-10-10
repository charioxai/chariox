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
    def test_mp07_builder_pin_substeps_survive_observer_shipping(self):
        events = tuple(sorted(event for event in diag.EVENTS if event.startswith('builder_pin_')))
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            guest, observer = pathlib.Path(a), pathlib.Path(b)
            for event in events:
                diag.event(guest, event)
            result = diag.ship(guest, 'https://observer.example/path1-diagnostics/round-e', set(),
                               lambda raw: {'sha256': diag.accept(observer, raw)})
            records = [json.loads(path.read_bytes()) for path in observer.glob('*.json')]
            self.assertEqual(result['records'], len(events))
            self.assertEqual({record['event'] for record in records}, set(events))
            for record in records:
                self.assertEqual(set(record), {'schema', 'atMs', 'pid', 'event'})
    def test_mp10_snapshot_hash_is_observer_verifiable_and_deduplicates_replay(self):
        import hashlib
        with tempfile.TemporaryDirectory() as a,tempfile.TemporaryDirectory() as b:
            guest,observer=pathlib.Path(a),pathlib.Path(b)
            first=diag.canonical({'schema':1,'atMs':1,'pid':20,'event':'heartbeat_sent'})
            second=None
            for stamp in range(2,100):
                value=diag.canonical({'schema':1,'atMs':stamp,'pid':30,'event':'cloud_presence_acknowledged'})
                if hashlib.sha256(first).hexdigest()>hashlib.sha256(value).hexdigest():second=value;break
            self.assertIsNotNone(second)
            for name,data in [('runtime-20-1.jsonl',first+first),('runtime-30-1.jsonl',second)]:
                path=guest/name;path.write_bytes(data);path.chmod(0o600)
            result=diag.ship(guest,'https://observer.example/path1-diagnostics/round-20261009',set(),lambda raw:{'sha256':diag.accept(observer,raw)})
            identifiers=sorted(path.stem for path in observer.glob('*.json'))
            self.assertEqual(result['records'],len(identifiers))
            self.assertEqual(result['snapshotSha256'],hashlib.sha256('\n'.join(identifiers).encode()).hexdigest())
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
    def campaign_preflight(self, root, mutate=None):
        # MP-07/MP-10: run installer primitives in an owned root mapping, then
        # the exact upgrade preflight. systemd is supplementary fixture wiring.
        import subprocess
        source_dir=pathlib.Path(__file__).resolve().parent
        installer=(source_dir/'enable-campaign-diagnostics.sh').read_text()
        for path in ('/etc/systemd/system','/home/chariox/.chariox','/usr/lib/chariox'):
            installer=installer.replace(path,str(root)+path)
        installer=installer.replace('Environment=CHARIOX_RUNTIME_DIAGNOSTICS_DIR='+str(root)+'/home/chariox/.chariox/runtime-diagnostics','Environment=CHARIOX_RUNTIME_DIAGNOSTICS_DIR=/home/chariox/.chariox/runtime-diagnostics')
        installer=installer.replace('-o chariox -g chariox','-o root -g root').replace('User=chariox','User=root').replace('Group=chariox','Group=root')
        tool=root/'usr/lib/chariox/slice-build-context/deploy/managed-kernel/runtime-diagnostics.py'
        tool.parent.mkdir(parents=True);tool.write_text('# fixture presence only')
        bin_dir=root/'bin';bin_dir.mkdir()
        systemctl=bin_dir/'systemctl'
        systemctl.write_text("""#!/bin/sh
case "$1" in
  is-active) exit 3 ;;
  show)
    case "$2" in
      --property=NeedDaemonReload) printf no ;;
      --property=DropInPaths)
        for file in "$CAMPAIGN_TEST_ROOT/etc/systemd/system/$4.d/"*.conf; do
          if [ -e "$file" ] || [ -L "$file" ]; then printf '%s ' "$file"; fi
        done ;;
    esac ;;
esac
""")
        systemctl.chmod(0o755)
        env=dict(os.environ,PATH=str(bin_dir)+':'+os.environ['PATH'],CAMPAIGN_TEST_ROOT=str(root))
        installed=subprocess.run(['/bin/sh','-s','--','https://observer.example/path1-diagnostics/round-20261009c'],input=installer,text=True,capture_output=True,env=env)
        self.assertEqual(installed.returncode,0,installed.stderr)
        if mutate:mutate(root)
        upgrade=(source_dir/'upgrade-image.sh').read_text()
        guard=upgrade.split('assert_path1_service_overrides() {',1)[1].split('\n}',1)[0]
        command='managed_provider_topology=path1\ninstall_root=$1\nscript_root=$2\nassert_path1_service_overrides() {'+guard+'\n}\nassert_path1_service_overrides\n'
        return subprocess.run(['/bin/sh','-c',command,'preflight',str(root),str(source_dir)],text=True,capture_output=True,env=env)
    def test_mp07_installed_campaign_diagnostics_pass_upgrade_preflight(self):
        with tempfile.TemporaryDirectory() as a:
            result=self.campaign_preflight(pathlib.Path(a))
            self.assertEqual(result.returncode,0,result.stderr)
    def test_mp11_diagnostics_do_not_admit_unrelated_dropins(self):
        def unrelated(root):
            (root/'etc/systemd/system/chariox-path1-managed-bootstrap.service.d/50-hardening.conf').write_text('[Service]\nProtectHome=yes\n')
        with tempfile.TemporaryDirectory() as a:
            result=self.campaign_preflight(pathlib.Path(a),unrelated)
            self.assertNotEqual(result.returncode,0)
            self.assertIn('has systemd drop-ins',result.stderr)
    def test_mp11_campaign_override_rejects_tampering_links_and_writable_files(self):
        def tamper(root):config(root).write_text('[Service]\nEnvironment=HOME=/tmp\n')
        def writable(root):config(root).chmod(0o666)
        def link(root):
            file=config(root);outside=root/'outside';file.rename(outside);file.symlink_to(outside)
        def hardlink(root):os.link(config(root),root/'outside')
        def parent(root):config(root).parent.chmod(0o777)
        def config(root):return root/'etc/systemd/system/chariox-path1-managed-bootstrap.service.d/path1-campaign-diagnostics.conf'
        for mutate in (tamper,writable,link,hardlink,parent):
            with self.subTest(mutate=mutate.__name__),tempfile.TemporaryDirectory() as a:
                result=self.campaign_preflight(pathlib.Path(a),mutate)
                self.assertNotEqual(result.returncode,0)
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
    def test_mp07_partial_observer_write_can_retry_and_ship_later_records(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            guest, observer = pathlib.Path(a), pathlib.Path(b)
            for event in ('heartbeat_sent', 'update_downloaded'):
                diag.event(guest, event)
            original = diag.os.fdopen
            class PartialWrite:
                def __init__(self, stream): self.stream = stream
                def __enter__(self): return self
                def __exit__(self, *args): return self.stream.__exit__(*args)
                def fileno(self): return self.stream.fileno()
                def write(self, data):
                    os.write(self.fileno(), data[:len(data) // 2])
                    raise OSError('injected storage exhaustion before complete payload')
            def opened(descriptor, mode):
                stream = original(descriptor, mode)
                return PartialWrite(stream) if mode == 'wb' else stream
            acknowledged = set()
            post = lambda raw: {'sha256': diag.accept(observer, raw)}
            with patch.object(diag.os, 'fdopen', side_effect=opened):
                with self.assertRaises(OSError):
                    diag.ship(guest, 'https://observer.example/path1-diagnostics/round-20261009', acknowledged, post)
            self.assertEqual(acknowledged, set())
            result = diag.ship(guest, 'https://observer.example/path1-diagnostics/round-20261009', acknowledged, post)
            self.assertEqual(result['records'], 2)
            self.assertEqual(len(list(observer.glob('*.json'))), 2)
            self.assertEqual(list(observer.glob('*.tmp')), [])

    def test_mp07_observer_crash_before_publication_can_retry_and_ship_later_records(self):
        import subprocess, sys
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            guest, observer = pathlib.Path(a), pathlib.Path(b)
            for event in ('heartbeat_sent', 'update_downloaded'):
                diag.event(guest, event)
            raw = next(diag.records(guest))
            crash = '''import importlib.util, os, pathlib, sys
spec = importlib.util.spec_from_file_location('diagnostics', sys.argv[1])
diag = importlib.util.module_from_spec(spec); spec.loader.exec_module(diag)
original = diag.os.fdopen
class InterruptedWrite:
    def __init__(self, stream): self.stream = stream
    def __enter__(self): return self
    def __exit__(self, *args): return self.stream.__exit__(*args)
    def fileno(self): return self.stream.fileno()
    def write(self, data):
        os.write(self.fileno(), data[:len(data) // 2]); os.fsync(self.fileno())
        os._exit(73)
def opened(descriptor, mode):
    stream = original(descriptor, mode)
    return InterruptedWrite(stream) if mode == 'wb' else stream
diag.os.fdopen = opened
diag.accept(pathlib.Path(sys.argv[2]), sys.argv[3].encode())
'''
            result = subprocess.run([sys.executable, '-I', '-c', crash, str(spec.origin), str(observer), raw.decode()], capture_output=True)
            self.assertEqual(result.returncode, 73)
            acknowledged = set()
            shipped = diag.ship(guest, 'https://observer.example/path1-diagnostics/round-20261009', acknowledged,
                                lambda value: {'sha256': diag.accept(observer, value)})
            self.assertEqual(shipped['records'], 2)
            self.assertEqual(len(acknowledged), 2)
            for receipt in observer.glob('*.json'):
                self.assertEqual(receipt.stat().st_nlink, 1)
                self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)

    def test_mp11_existing_observer_corruption_and_links_still_fail_closed(self):
        import hashlib
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            observer, outside = pathlib.Path(a), pathlib.Path(b) / 'record'
            raw = diag.canonical({'schema': 1, 'atMs': 1, 'pid': 20, 'event': 'heartbeat_sent'})
            receipt = observer / (hashlib.sha256(raw).hexdigest() + '.json')
            receipt.write_bytes(raw[:5]); receipt.chmod(0o600)
            with self.assertRaises(ValueError): diag.accept(observer, raw)
            self.assertEqual(receipt.read_bytes(), raw[:5])
            receipt.unlink(); outside.write_bytes(raw); outside.chmod(0o600)
            receipt.symlink_to(outside)
            with self.assertRaises(OSError): diag.accept(observer, raw)
            receipt.unlink(); os.link(outside, receipt)
            with self.assertRaises(ValueError): diag.accept(observer, raw)
            self.assertEqual(outside.read_bytes(), raw)

    def test_mp07_preparation_is_observed_before_install_and_failure_is_shippable(self):
        import subprocess
        helper=pathlib.Path(__file__).with_name('image-preparation-progress.sh')
        events=['image_prepare_start','image_prepare_packages','image_install_start',
                'image_install_verify','image_install_pin','image_install_publish',
                'image_install_activate','image_install_complete','image_prepare_pin',
                'image_prepare_providers','image_prepare_provider_probe','image_prepare_rootless',
                'image_prepare_pull','image_prepare_build','image_prepare_freeze',
                'image_prepare_complete','image_prepare_failed']
        with tempfile.TemporaryDirectory() as a,tempfile.TemporaryDirectory() as b:
            guest,observer=pathlib.Path(a),pathlib.Path(b)
            env=dict(os.environ,CHARIOX_IMAGE_PREPARATION_DIAGNOSTICS_DIR=a)
            command='script_root=${1%/*}; . "$1"; shift; for event do record_image_preparation_phase "$event"; done'
            result=subprocess.run(['sh','-c',command,'test',str(helper),*events],env=env,text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(result.stdout.splitlines(),['chariox-image-preparation: '+e for e in events])
            rows=list(diag.records(guest))
            self.assertEqual([json.loads(r)['event'] for r in rows],events)
            receipt=diag.ship(guest,'https://observer.example/path1-diagnostics/round-20261010h',set(),lambda raw:{'sha256':diag.accept(observer,raw)})
            self.assertEqual(receipt['records'],len(events))
            result=subprocess.run(['sh','-c','. "$1"; record_image_preparation_phase "PRIVATE-canary"','test',str(helper)],env=env,text=True,capture_output=True)
            self.assertNotEqual(result.returncode,0)
            self.assertNotIn('PRIVATE-canary',result.stdout+result.stderr)
            self.assertEqual(len(list(diag.records(guest))),len(events))

    def test_mp07_preparation_progress_without_diagnostics_needs_no_node_or_python(self):
        import subprocess
        helper=pathlib.Path(__file__).with_name('image-preparation-progress.sh')
        result=subprocess.run(['/bin/sh','-c','. "$1"; record_image_preparation_phase image_prepare_packages','test',str(helper)],env={'PATH':'/nonexistent'},text=True,capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(result.stdout,'chariox-image-preparation: image_prepare_packages\n')

    def test_mp07_observed_wrapper_keeps_preparation_exit_and_public_terminal_phase(self):
        import subprocess
        source=pathlib.Path(__file__).with_name('observe-image-preparation.sh')
        helper=pathlib.Path(__file__).with_name('image-preparation-progress.sh')
        for code in (0,42):
            with tempfile.TemporaryDirectory() as a:
                root=pathlib.Path(a)
                marker=root/'builder-marker';marker.write_text('managed-remote-kernels-image-builder-v1\n')
                wrapper=root/'observe-image-preparation.sh'
                wrapper.write_text(source.read_text().replace('/.chariox-managed-image-builder',str(marker)))
                (root/helper.name).write_bytes(helper.read_bytes())
                (root/'prepare-hetzner-image.sh').write_text('exit '+str(code)+'\n')
                result=subprocess.run(['/bin/sh',str(wrapper),'unused-rootfs','unused-digest','unused-public-pin'],env={'PATH':'/usr/bin:/bin'},text=True,capture_output=True)
                self.assertEqual(result.returncode,code,result.stderr)
                self.assertEqual(result.stdout.splitlines(),['chariox-image-preparation: image_prepare_start',
                    'chariox-image-preparation: image_prepare_'+('complete' if code==0 else 'failed')])

if __name__=='__main__':unittest.main()
