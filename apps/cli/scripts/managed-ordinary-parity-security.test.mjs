import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { promisify } from 'node:util';
import test from 'node:test';
import { observeDirectoryCheck, observeWorkspaceCheck } from './managed-ordinary-parity-probe.mjs';
import { normalizeGenericResult, normalizeProviderAncestry } from './managed-ordinary-parity-collector.mjs';
const exec = promisify(execFile);

test('MP-11 forged privilege evidence cannot override live NoNewPrivs=1', async () => {
  const module = new URL('./managed-ordinary-parity-probe.mjs', import.meta.url).href;
  const child = `import {observePrivilegeState} from ${JSON.stringify(module)};
    try { await observePrivilegeState({}); console.log('ACCEPTED'); } catch { console.log('REJECTED'); }`;
  const status = await readFile('/proc/self/status', 'utf8');
  const baseline = { cap_eff: /^CapEff:\s*(\S+)/m.exec(status)[1], umask: process.umask().toString(8).padStart(4,'0'), uid: process.getuid(), gid: process.getgid() };
  const {stdout} = await exec('setpriv', ['--no-new-privs', process.execPath, '--input-type=module', '-e', child], {env:{...process.env,
    CHARIOX_PARITY_ORDINARY_PRIVILEGE_JSON: JSON.stringify(baseline),
    CHARIOX_PARITY_PRIVILEGE_EVIDENCE_JSON: JSON.stringify({observed:true, no_new_privs:false}),
  }});
  assert.equal(stdout.trim(), 'REJECTED');
});

test('MP-11 forged ancestry evidence cannot suppress observed bwrap', async () => {
  const module = new URL('./managed-ordinary-parity-probe.mjs', import.meta.url).href;
  const child = `import {observeProviderAncestry} from ${JSON.stringify(module)};
    // bwrap is a synthetic command-line ancestry marker, never a namespace.
    try { await observeProviderAncestry({}, {provider:'codex'}); console.log('ACCEPTED'); } catch { console.log('REJECTED'); }`;
  const {stdout} = await exec(process.execPath, ['--input-type=module','-e',child], {env:{...process.env,
    CHARIOX_PARITY_WORKER_EVIDENCE_JSON: JSON.stringify({observed:true, fresh_worker:true}),
    CHARIOX_PARITY_ANCESTRY_EVIDENCE_JSON: JSON.stringify({observed:true, provider_observed:true,bwrap_ancestor:false,fresh_worker:true,ancestry_complete:true}),
  }});
  assert.equal(stdout.trim(), 'REJECTED');
});

test('MP-11 normalizers reject contradictory observed boundary facts', () => {
  assert.throws(() => normalizeGenericResult({observed:true,no_new_privs:false,observed_no_new_privs:true,capabilities_match_ordinary:true,umask_matches_ordinary:true}, 'MP-01','privilege_state'));
  assert.throws(() => normalizeProviderAncestry({observed:true,provider_observed:true,bwrap_ancestor:false,observed_bwrap_ancestor:true,fresh_worker:true,ancestry_complete:true}));
});

for (const check of ['directory_creation', 'empty_workspace', 'basename_collision']) {
  for (const layout of ['populated', 'symlink']) {
    test(`MP-11 ${check} preserves a foreign ${layout} target`, async () => {
      const root = await mkdtemp('/tmp/chariox-parity-mp11-test-');
      try {
        const foreign = join(root, 'foreign');
        const target = join(root, 'requested');
        await mkdir(foreign);
        await writeFile(join(foreign,'user-content'), 'user data');
        if(layout === 'symlink') await symlink(foreign,target);
        else { await mkdir(target); await writeFile(join(target,'user-content'), 'user data'); }
        const values = {new_directory:target,nested_path:target,source_root:root};
        const operation = check === 'directory_creation' ? observeDirectoryCheck : observeWorkspaceCheck;
        await assert.rejects(operation({}, values, check));
        assert.equal(await readFile(join(target,'user-content'),'utf8'),'user data');
        assert.equal(await readFile(join(foreign,'user-content'),'utf8'),'user data');
      } finally { await rm(root,{recursive:true,force:true}); }
    });
  }
}
