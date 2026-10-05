import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {spawnSync} from 'node:child_process'
const repo=process.env.MP11_TEST_SOURCE??path.resolve(import.meta.dirname,'..')
function driver(root,action){
 const source=fs.readFileSync(path.join(repo,'apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh'),'utf8')
 const copy=source.slice(source.indexOf('copy_provider_auth_file() {'),source.indexOf('trust_claude_slice_workspace() {'))
 const remove=source.slice(source.indexOf('remove_codex_auth() {'),source.indexOf('import_opencode_auth() {'))
 return spawnSync('bash',['-euc',`${copy}\n${remove}\nlog(){ :; };run_with_file_stdin_timeout(){ local seconds="$1" source="$2";shift 2;"$@" < "$source"; };docker(){ bash -lc "\${@: -1}"; };exec_slice(){ "$@"; };\n${action}`],{timeout:3000,encoding:'utf8',env:{PATH:process.env.PATH,SCRIPT_DIR:path.join(repo,'apps/kernel/slice-linux-docker'),SLICE_NAME:'synthetic',SLICE_ACCOUNT_ROOT:root,SLICE_ACCOUNT_PROFILE:'fixture',SOURCE:path.join(root,'source'),TARGET:path.join(root,'codex/fixture/codex/auth.json')}})
}
test('MP-11 F18 auth removal retires both product-owned credential copies',()=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'mp11-slice-auth-'))
 try{const target=path.join(root,'codex/fixture/codex/auth.json');fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,'synthetic-prior');fs.writeFileSync(path.join(root,'source'),'synthetic-incoming',{mode:0o600})
 const run=driver(root,'copy_provider_auth_file "$SOURCE" "$TARGET" fixture; remove_codex_auth');assert.equal(run.status,0)
 assert.equal(fs.existsSync(target),false);assert.equal(fs.existsSync(`${target}.before-slice-auth`),false)
 }finally{fs.rmSync(root,{recursive:true,force:true})}
})
test('MP-11 F18 auth publication refuses an aliased destination before modifying unrelated bytes',()=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'mp11-slice-auth-'))
 try{const target=path.join(root,'codex/fixture/codex/auth.json');fs.mkdirSync(path.dirname(target),{recursive:true});const unrelated=path.join(root,'unrelated');fs.writeFileSync(unrelated,'synthetic-unrelated',{mode:0o644});fs.symlinkSync(unrelated,target);fs.writeFileSync(path.join(root,'source'),'synthetic-incoming',{mode:0o600})
 const run=driver(root,'copy_provider_auth_file "$SOURCE" "$TARGET" fixture');assert.notEqual(run.status,0)
 assert.equal(fs.readFileSync(unrelated,'utf8')==='synthetic-unrelated',true);assert.equal(fs.statSync(unrelated).mode&0o777,0o644)
 }finally{fs.rmSync(root,{recursive:true,force:true})}
})
