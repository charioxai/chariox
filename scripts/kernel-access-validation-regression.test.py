#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: runner provenance and actual runner SIGTERM teardown.

The small temporary Git repository and command stubs test runner control flow;
they do not count as kernel/provider acceptance. No Rust build or account is used.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SOURCE = Path(os.environ.get('KASUDO_VALIDATION_SCRIPT', str(Path(__file__).with_name('kernel-access-validation.py'))))
spec = importlib.util.spec_from_file_location('validation', SOURCE)
validation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validation)


class RunnerRegressionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='kasudo-runner-test-')
        self.base = Path(self.tmp.name)
        self.repo = self.base / 'source'
        self.repo.mkdir()
        scripts = self.repo / 'scripts'
        scripts.mkdir()
        shutil.copyfile(SOURCE, scripts / SOURCE.name)
        for name in ['kernel-access-validation.test.py', 'sudo-kit.mjs', 'sudo-kit-smoke.mjs', 'sudo-kit-receipts.py']:
            (scripts / name).write_text('')
        (self.repo / 'authority.rs').write_text('// synthetic authorization source\n')
        fakebin = self.repo / 'fixture-bin'
        fakebin.mkdir()
        node = fakebin / 'node'
        node.write_text('''#!/usr/bin/env python3
import json, os, subprocess, time
from pathlib import Path
mode = os.environ.get("KASUDO_FIXTURE_MODE", "quick")
if mode == "mutate":
    with Path("authority.rs").open("a") as f: f.write("// source changed during stage\\n")
if mode == "wait":
    child = subprocess.Popen(["/bin/sleep", "30"])
    Path(os.environ["KASUDO_FIXTURE_MARKER"]).write_text(json.dumps({"node":os.getpid(), "child":child.pid}))
    child.wait()
''')
        node.chmod(0o755)
        artifact = self.base / 'synthetic-test-artifact'
        artifact.write_text('#!/usr/bin/env python3\nprint("test result: ok. 1 passed; 0 failed; 0 ignored")\n')
        artifact.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update(PATH=str(fakebin) + ':' + self.env['PATH'], PYTHONDONTWRITEBYTECODE='1',
                        GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null',
                        KASUDO_FIXTURE_MARKER=str(self.base/'marker.json'))
        self.git('init', '-q')
        self.git('add', '.')
        self.git('-c', 'user.name=Chariox regression fixture', '-c', 'user.email=noreply@openai.com',
                 '-c', 'commit.gpgsign=false', 'commit', '-qm', 'MP-11 synthetic runner fixture [skip ci]\n\nCo-Authored-By: GPT-6.1-sol (Codex) <noreply@openai.com>')
        self.receipt = self.base/'receipt.json'
        self.receipt.write_text(json.dumps({'mp':['MP-08','MP-10','MP-11'],
            'head': self.git('rev-parse','HEAD').strip(), 'tree': self.git('rev-parse','HEAD^{tree}').strip(),
            'source_policy':'clean_checkout', 'binary':str(artifact),
            'sha256':hashlib.sha256(artifact.read_bytes()).hexdigest()}))
        self.owned = {}

    def git(self, *args):
        return subprocess.check_output(['git', *args],cwd=self.repo,env=self.env,text=True)

    def tearDown(self):
        validation.stop_owned(self.owned)
        evidence = os.environ.get('KASUDO_REGRESSION_EVIDENCE')
        if evidence and (self.base/'output').exists():
            shutil.copytree(self.base/'output', Path(evidence)/self._testMethodName)
        self.tmp.cleanup()

    def start(self, mode='quick', extra_args=()):
        self.env['KASUDO_FIXTURE_MODE']=mode
        # Tiny synthetic stages use their own lock and explicit fixture resource floors.
        proc=subprocess.Popen([sys.executable,str(self.repo/'scripts'/SOURCE.name), '--kit-only',
            '--output',str(self.base/'output'), '--artifact-receipt',str(self.receipt),
            '--compile-lock',str(self.base/'compile.lock'), '--min-available-gib','0',
            '--min-disk-free-gib','0', *extra_args],
            env=self.env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,start_new_session=True)
        row=validation.identity(proc.pid)
        if row: self.owned[proc.pid]=row[1]
        return proc

    def run_fixture(self, mode='quick', extra_args=()):
        proc=self.start(mode, extra_args)
        try:
            output=proc.communicate(timeout=35)[0]
            return proc.returncode,output
        finally:
            validation.collect_owned(self.owned)
            validation.stop_owned(self.owned)
            proc.wait(timeout=5)

    def test_clean_checkout_can_run_fixture_kit(self):
        code,output=self.run_fixture()
        self.assertEqual(code,0,output)
        checks=[json.loads(line) for line in (self.base/'output/checks.jsonl').read_text().splitlines()]
        self.assertEqual([check['name'] for check in checks],
                         ['watchdog','popup-client','kit-smoke','sandboxed-pty-child'])
        for check in checks[-2:]:
            self.assertEqual(check['command'][:2],['flock',str(self.base/'compile.lock')])
        cleanup=json.loads((self.base/'output/cleanup.json').read_text())
        self.assertTrue(cleanup['state_removed'])
        self.assertEqual(cleanup['remaining_owned_processes'],[])

    def test_configured_resource_floor_refuses_stage_and_cleans_state(self):
        for option in ['--min-available-gib','--min-disk-free-gib']:
            with self.subTest(option=option):
                code,output=self.run_fixture(extra_args=[option,'1000000000'])
                self.assertNotEqual(code,0,output)
                self.assertIn('resource floor before watchdog',output)
                cleanup=json.loads((self.base/'output/cleanup.json').read_text())
                self.assertTrue(cleanup['state_removed'])
                self.assertEqual(cleanup['results'],0)
                shutil.rmtree(self.base/'output')

    def test_invalid_resource_floors_are_rejected_before_state_creation(self):
        for option in ['--min-available-gib','--min-disk-free-gib']:
            for value in ['nan','inf','-1']:
                with self.subTest(option=option,value=value):
                    code,output=self.run_fixture(extra_args=[option,value])
                    self.assertNotEqual(code,0,output)
                    self.assertIn('resource floors must be finite and nonnegative',output)
                    self.assertFalse((self.base/'output').exists())

    def test_unstaged_source_is_refused_before_commands(self):
        (self.repo/'authority.rs').write_text('// uncommitted source\n')
        code,output=self.run_fixture()
        self.assertNotEqual(code,0,output)
        self.assertIn('source',output.lower())
        self.assertFalse((self.base/'output/checks.jsonl').exists())

    def test_staged_source_is_refused_before_commands(self):
        (self.repo/'authority.rs').write_text('// staged source\n')
        self.git('add','authority.rs')
        code,output=self.run_fixture()
        self.assertNotEqual(code,0,output)

    def test_untracked_source_is_refused_before_commands(self):
        (self.repo/'new-authority.rs').write_text('// untracked source\n')
        code,output=self.run_fixture()
        self.assertNotEqual(code,0,output)

    def test_stage_source_mutation_cannot_report_success(self):
        code,output=self.run_fixture('mutate')
        self.assertNotEqual(code,0,output)
        self.assertIn('source',output.lower())
        checks=[json.loads(line) for line in (self.base/'output/checks.jsonl').read_text().splitlines()]
        self.assertNotEqual(checks[-1]['exit'],0)

    def test_kit_receipt_tree_must_match_the_current_source(self):
        receipt=json.loads(self.receipt.read_text())
        receipt['tree']='0'*40
        self.receipt.write_text(json.dumps(receipt))
        code,output=self.run_fixture()
        self.assertNotEqual(code,0,output)

    def test_kit_receipt_requires_clean_source_proof(self):
        receipt=json.loads(self.receipt.read_text())
        receipt.pop('source_policy')
        self.receipt.write_text(json.dumps(receipt))
        code,output=self.run_fixture()
        self.assertNotEqual(code,0,output)

    def test_operator_handoffs_are_excluded_from_source(self):
        for name in ['LANE_STATUS.md','PUSH_READY.md']:
            (self.repo/name).write_text('MP-11 public operator handoff\n')
        code,output=self.run_fixture()
        self.assertEqual(code,0,output)

    def test_sigterm_runner_records_cancellation_and_stops_its_children(self):
        proc=self.start('wait')
        try:
            deadline=time.monotonic()+15
            marker=self.base/'marker.json'
            while not marker.exists() and proc.poll() is None and time.monotonic()<deadline:
                time.sleep(.1)
            self.assertTrue(marker.exists(),'runner must reach its detached command stage')
            for pid in json.loads(marker.read_text()).values():
                row=validation.identity(pid)
                if row: self.owned[pid]=row[1]
            time.sleep(2.5)  # Let the actual runner's watchdog observe descendants.
            row=validation.identity(proc.pid)
            self.assertIsNotNone(row)
            self.assertGreater(proc.pid,1)
            self.assertEqual(row[1],self.owned[proc.pid])
            fd=os.pidfd_open(proc.pid)
            try:
                current=validation.identity(proc.pid)
                if current and current[1]==row[1] and isinstance(proc.pid,int) and proc.pid>1:
                    signal.pidfd_send_signal(fd,signal.SIGTERM)
            finally:
                os.close(fd)
            output=proc.communicate(timeout=15)[0]
            self.assertNotEqual(proc.returncode,0,output)
            cleanup_path=self.base/'output/cleanup.json'
            self.assertTrue(cleanup_path.exists(),'SIGTERM must execute outer cleanup')
            cleanup=json.loads(cleanup_path.read_text())
            self.assertEqual(cleanup['cancelled_signal'],signal.SIGTERM)
            self.assertTrue(cleanup['state_removed'])
            self.assertEqual(cleanup['remaining_owned_processes'],[])
            self.assertFalse(Path(cleanup['state']).exists())
            for pid,birth in self.owned.items():
                current=validation.identity(pid)
                self.assertFalse(current and current[1]==birth and current[2]!='Z',f'owned process {pid} remains')
        finally:
            validation.stop_owned(self.owned)
            proc.wait(timeout=5)


if __name__=='__main__':
    unittest.main()
