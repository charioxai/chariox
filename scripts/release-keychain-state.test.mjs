import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
const source = process.env.MP11_TEST_SOURCE ?? path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
function setupText() {
  const workflow = fs.readFileSync(path.join(source, '.github/workflows/release-bundle.yml'), 'utf8')
  const start = workflow.indexOf('      - name: Load the Developer ID')
  const block = workflow.slice(start, workflow.indexOf('      # #678:', start))
  return block.slice(block.indexOf('        run: |\n') + 15).split('\n').map(line => line.replace(/^          /, '')).join('\n')
}
for (const stage of ['create-keychain', 'set-keychain-settings', 'unlock-keychain', 'import', 'set-key-partition-list', 'search-new', 'default-new', 'notary']) {
  test(`MP-11 F19 setup failure at ${stage} restores keychain state`, () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'mp11-keychain-'))
    try {
      const bin = path.join(root, 'bin'); fs.mkdirSync(bin)
      const stub = `#!/usr/bin/env node
const fs=require('fs');const path=require('path');const args=process.argv.slice(2);const root=process.env.RUNNER_TEMP;
let stage=args[0];
if(stage==='list-keychains' && !args.includes('-s')) { console.log('    "/synthetic/login.keychain-db"\\n    "/synthetic/other.keychain-db"');process.exit(0) }
if(stage==='default-keychain' && !args.includes('-s')) { console.log('    "/synthetic/login.keychain-db"');process.exit(0) }
if(stage==='list-keychains') stage=args.includes('/synthetic/login.keychain-db')?'search-restored':'search-new';
if(stage==='default-keychain') stage=args.includes('/synthetic/login.keychain-db')?'default-restored':'default-new';
fs.appendFileSync(path.join(root,'calls'),stage+'\\n');
if(stage==='create-keychain')fs.writeFileSync(path.join(root,'release.keychain-db'),'synthetic');
if(stage==='delete-keychain')fs.rmSync(path.join(root,'release.keychain-db'),{force:true});
if(stage===process.env.FAIL_STAGE)process.exit(1);
`
      fs.writeFileSync(path.join(bin, 'security'), stub, { mode: 0o700 })
      fs.writeFileSync(path.join(bin, 'uuidgen'), '#!/bin/sh\nprintf synthetic-password\n', { mode: 0o700 })
      fs.writeFileSync(path.join(bin, 'xcrun'), '#!/bin/sh\nprintf "notary\\n" >> "$RUNNER_TEMP/calls"\n[ "$FAIL_STAGE" != notary ]\n', { mode: 0o700 })
      const run = spawnSync('bash', ['-euo', 'pipefail', '-c', setupText()], { cwd: source, encoding: 'utf8', env: {
        PATH: `${bin}:${process.env.PATH}`, RUNNER_TEMP: root, GITHUB_ENV: path.join(root,'github-env'), FAIL_STAGE: stage,
        P12: Buffer.from('synthetic').toString('base64'), P12_PASSWORD: 'synthetic', NOTARY_KEY: 'synthetic',
        NOTARY_KEY_ID: 'synthetic', NOTARY_ISSUER: 'synthetic', CHARIOX_NOTARY_PROFILE: 'synthetic',
      } })
      assert.notEqual(run.status, 0)
      const calls = fs.readFileSync(path.join(root,'calls'),'utf8')
      assert.ok(calls.includes('delete-keychain'))
      assert.ok(calls.includes('search-restored'))
      assert.ok(calls.includes('default-restored'))
      for (const name of ['release.keychain-db', 'identity.p12', 'notary.p8']) assert.equal(fs.existsSync(path.join(root,name)),false)
    } finally { fs.rmSync(root,{recursive:true,force:true}) }
  })
}
