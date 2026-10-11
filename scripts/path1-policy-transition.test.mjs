// MP-03/MP-07/MP-11: immutable historical policies and a two-hop signed transition.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { verifyPath1ServicePolicy } from '../deploy/managed-kernel/path1-service-policy.mjs'
const home = (await readFile(new URL('../deploy/managed-kernel/chariox-path1-managed-bootstrap.service', import.meta.url), 'utf8')).replace('CHARIOX_PATH1_SERVICE_POLICY_VERSION=2','CHARIOX_PATH1_SERVICE_POLICY_VERSION=1')
let worker = (await readFile(new URL('../deploy/managed-kernel/chariox-disposable-worker-bootstrap.service', import.meta.url), 'utf8')).replace('CHARIOX_PATH1_SERVICE_POLICY_VERSION=2','CHARIOX_PATH1_SERVICE_POLICY_VERSION=1')
if (!worker.includes('StateDirectory=')) worker = worker.replace('KillMode=control-group','KillMode=control-group\nStateDirectory=chariox\nStateDirectoryMode=0700')
const v2 = unit => unit.replace(/^Environment=CHARIOX_PATH1_SERVICE_POLICY_VERSION=.*\n/gm, '').replace(/^StateDirectory(?:Mode)?=.*\n/gm, '').replace('[Service]', '[Service]\nEnvironment=CHARIOX_PATH1_SERVICE_POLICY_VERSION=2')
for (const [round, head] of [['H','ece7e0fc621d413fea8a804caaf74772148ea0a0'], ['I','a277240c7cdba6201dc49e104e18d537de37ffc0']]) {
  const source = execFileSync('git', ['show', `${head}:deploy/managed-kernel/path1-service-policy.mjs`], {encoding:'utf8'})
  const old = await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'))
  const legacy = execFileSync('git', ['show', `${head}:deploy/managed-kernel/chariox-disposable-worker-bootstrap.service`], {encoding:'utf8'})
  test(`MP-07 ${round} actual policy admits K-A compatibility units`, () => {
    old.verifyPath1ServicePolicy(home,'home')
    old.verifyPath1ServicePolicy(worker,'worker')
    verifyPath1ServicePolicy(legacy,'worker')
  })
  test(`MP-03 ${round} requires the K-A bridge before policy 2`, () => {
    assert.throws(() => old.verifyPath1ServicePolicy(v2(worker), 'worker'), /StateDirectory/)
    verifyPath1ServicePolicy(v2(worker), 'worker')
    verifyPath1ServicePolicy(v2(home), 'home')
    assert.throws(() => verifyPath1ServicePolicy(v2(worker)+'\n[Service]\nStateDirectory=chariox\nStateDirectoryMode=0700\n','worker'), /StateDirectory/)
  })
}
test('MP-03 unknown and overridden signed policy versions fail closed', () => {
  for (const version of ['0','3','02','', '2\nEnvironment=CHARIOX_PATH1_SERVICE_POLICY_VERSION=1']) {
    assert.throws(() => verifyPath1ServicePolicy(v2(home).replace('VERSION=2','VERSION='+version), 'home'), /policy version/)
  }
})

test('MP-03 quoted or combined policy markers cannot hide a v2 override', () => {
  const bare = worker.replace(/^Environment=CHARIOX_PATH1_SERVICE_POLICY_VERSION=.*\n/gm, '')
  for (const marker of ['Environment="CHARIOX_PATH1_SERVICE_POLICY_VERSION=2"', 'Environment=OTHER=1 CHARIOX_PATH1_SERVICE_POLICY_VERSION=2', 'Environment=CHARIOX_PATH1_SERVICE_POLICY_VERSION="2"']) {
    assert.throws(() => verifyPath1ServicePolicy(bare.replace('[Service]', '[Service]\n'+marker), 'worker'), /policy version/)
  }
})
