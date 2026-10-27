"""MP-08 / MP-10 / MP-11: ordinary publications remain usable by both kernels."""
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
UID = GID = 31001


@unittest.skipUnless(os.geteuid() == 0, "needs an isolated worker UID")
class DevelopmentWorkspaceAccess(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="chariox-development-access-"))
        self.root.chmod(0o755)
        self.private = self.root / "private-parent"
        self.private.mkdir(mode=0o700)
        self.workspace = self.private / "published"
        self.workspace.mkdir(mode=0o700)
        self.file = self.workspace / "config.json"
        self.file.write_text("synthetic config")
        self.file.chmod(0o600)
        self.outside = self.root / "outside"
        self.outside.write_text("outside sentinel")
        self.outside.chmod(0o600)
        (self.workspace / "outside-link").symlink_to(self.outside)
        self.outside_stat = self.outside.stat()
        self.docker = self.root / "docker"
        self.docker.write_text('''#!/usr/bin/env python3
import os,pwd,sys,types
assert sys.argv[1:7] == ['exec','-i','-u','root','fixture','python3'], sys.argv
sys.argv=['-']+sys.argv[8:]
pwd.getpwnam=lambda name: types.SimpleNamespace(pw_uid=31001,pw_gid=31001)
exec(compile(sys.stdin.read(), '<container helper>', 'exec'), {'__name__':'__main__'})
''')
        self.docker.chmod(0o755)

    def tearDown(self):
        shutil.rmtree(self.root)

    def prepare(self, managed=False, fail=False, paths=None):
        source = (HERE / "provision-linux-docker-slice.sh").read_text()
        function = re.search(r"^prepare_development_workspace_access\(\) \{[\s\S]*?^\}", source, re.M)
        # Frozen D has no publication-access step; reproduce its actual no-op.
        script = (function.group() if function else "prepare_development_workspace_access() { :; }")
        paths = paths or [self.workspace]
        script += '\nrun_with_timeout() { shift; "$@"; }\nfail() { echo "$*" >&2; exit 1; }\n'
        script += 'prepare_development_workspace_access\n'
        env = {k:v for k,v in os.environ.items() if not k.startswith('CHARIOX_') and k != 'BASH_ENV'}
        env.update(PATH=str(self.root)+":"+env['PATH'], SCRIPT_DIR=str(HERE),
                   SLICE_NAME='fixture', SLICE_DEVELOPMENT_MOUNT_COUNT=str(len(paths)),
                   CHARIOX_SLICE_MANAGED_DOCKER_HOST='synthetic' if managed else '')
        for index,path in enumerate(paths):
            env['CHARIOX_SLICE_DEVELOPMENT_MOUNT_'+str(index)] = str(path)
        if fail:
            self.docker.write_text('#!/bin/sh\nexit 43\n')
        return subprocess.run(['bash','-c',script],env=env,capture_output=True,text=True,timeout=10)

    def worker(self):
        script = '''import os,sys
p=sys.argv[1]
with open(p) as f: assert f.read() == 'synthetic config'
with open(p,'w') as f: f.write('target edit')
with open(os.path.join(os.path.dirname(p),'new-file'),'w') as f: f.write('target-owned')
'''
        def identity():
            os.setgroups([])
            os.setgid(GID)
            os.setuid(UID)
        return subprocess.run(['python3','-c',script,str(self.file)],preexec_fn=identity,capture_output=True,text=True)

    def test_private_ancestor_and_owner_only_files_are_usable_without_changing_owner(self):
        self.assertNotEqual(self.worker().returncode,0)
        result=self.prepare()
        self.assertEqual(result.returncode,0,result.stderr)
        result=self.worker()
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(self.file.read_text(),'target edit')
        self.assertEqual(self.file.stat().st_uid,0,'source owner must retain access')
        self.assertEqual(self.file.stat().st_mode & 7,0,'no world access')
        self.assertEqual(self.private.stat().st_mode & 0o777,0o710,'ancestor search only')
        self.assertEqual(self.outside.stat(),self.outside_stat,'symlink target untouched')

    def test_managed_broker_keeps_its_existing_publication_access_contract(self):
        before=self.file.stat()
        result=self.prepare(managed=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(self.file.stat(),before)
        self.assertNotEqual(self.worker().returncode,0)

    def test_failed_access_preparation_stops_runtime_startup(self):
        result=self.prepare(fail=True)
        self.assertNotEqual(result.returncode,0)
        self.assertIn('development workspace access',result.stderr)

    def test_symlink_publication_root_is_rejected(self):
        link=self.root/'root-link'
        link.symlink_to(self.workspace)
        result=self.prepare(paths=[link])
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(self.file.stat().st_mode & 0o777,0o600)

    def test_invalid_supporting_root_does_not_change_primary_access(self):
        result = self.prepare(paths=[self.workspace, self.root / 'missing'])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.file.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.private.stat().st_mode & 0o777, 0o700)

    def test_primary_and_supporting_repositories_are_shared(self):
        supporting = self.private / 'supporting'
        supporting.mkdir(mode=0o700)
        config = supporting / 'config.json'
        config.write_text('synthetic config')
        config.chmod(0o600)
        result = self.prepare(paths=[self.workspace, supporting])
        self.assertEqual(result.returncode, 0, result.stderr)
        for file in [self.file, config]:
            self.file = file
            result = self.worker()
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_unprivileged_source_owner_retains_private_file_access(self):
        source_uid = source_gid = 31002
        for path in [self.private, self.workspace, self.file]:
            os.chown(path, source_uid, source_gid)
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.worker()
        self.assertEqual(result.returncode, 0, result.stderr)

        def source_identity():
            os.setgroups([])
            os.setgid(source_gid)
            os.setuid(source_uid)

        result = subprocess.run(
            ['python3', '-c', 'import pathlib,sys; assert pathlib.Path(sys.argv[1]).read_text() == "target edit"', str(self.file)],
            preexec_fn=source_identity, capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.file.stat().st_uid, source_uid)


if __name__ == '__main__':
    unittest.main()
