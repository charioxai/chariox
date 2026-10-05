import test from 'node:test'
import assert from 'node:assert/strict'
import { remoteEnvCommand } from '../apps/cli/scripts/lib/native-tui-remote-execution.mjs'
test('MP-11 F26 legacy argv launcher refuses credential-bearing environment', () => {
  assert.throws(() => remoteEnvCommand({CHARIOX_REMOTE_REPO:'/synthetic',CHARIOX_RELAY_TOKEN:'synthetic-secret'},'true'), /private stdin/)
})

test('MP-11 F26 remote argv and SSH environment exclude synthetic secrets', async () => {
  const { spawnRemoteEnv } = await import('../apps/cli/scripts/lib/native-tui-remote-execution.mjs')
  const { PassThrough } = await import('node:stream')
  const sentinel = 'mp11-synthetic-relay-secret'
  const input = new PassThrough(); let payload = ''
  input.on('data', chunk => { payload += chunk })
  let captured
  const child = spawnRemoteEnv({hetznerHost:'synthetic',hetznerKey:'/synthetic/public-fixture'}, {CHARIOX_REMOTE_REPO:'/synthetic',CHARIOX_RELAY_TOKEN:sentinel}, 'true', {}, (command,args,options) => {
    captured = {command,args,options}; return {stdin: input}
  })
  assert.ok(child)
  assert.equal(JSON.stringify(captured).includes(sentinel), false)
  assert.equal(JSON.parse(payload).environment.CHARIOX_RELAY_TOKEN,sentinel)
  assert.equal(captured.options.stdio[0],'pipe')
  assert.equal(captured.options.stdio[2],'ignore')
})

test('MP-11 F26 PTY spawn environment excludes unrelated provider credentials', async () => {
  const { privateClientEnvironment } = await import('../apps/cli/scripts/lib/native-tui-remote-execution.mjs')
  const result = privateClientEnvironment({PATH:'/synthetic',CHARIOX_RELAY_TOKEN:'synthetic-scoped',DATABASE_URL:'synthetic-private',OPENAI_API_KEY:'synthetic-private'})
  assert.deepEqual(result,{PATH:'/synthetic',CHARIOX_RELAY_TOKEN:'synthetic-scoped'})
})

test('MP-11 F26 fixed remote launcher receives stdin and clears inherited environment', async () => {
  const {spawnRemoteEnv} = await import('../apps/cli/scripts/lib/native-tui-remote-execution.mjs')
  const {spawn} = await import('node:child_process')
  const child = spawnRemoteEnv({hetznerHost:'fixture',hetznerKey:'/fixture'}, {CHARIOX_REMOTE_REPO:'/tmp',CHARIOX_RELAY_TOKEN:'synthetic'},
    `python3 -c 'import os;print(os.getenv("CHARIOX_RELAY_TOKEN")=="synthetic",os.getenv("DATABASE_URL") is None)'`, {stdio:['pipe','pipe','ignore']},
    (_command,args,options) => spawn('bash',['-c',args.at(-1)],{...options,env:{...options.env,DATABASE_URL:'synthetic-unrelated'}}))
  let output = '';child.stdout.on('data',chunk=>{output += chunk})
  const status = await new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',resolve)})
  assert.equal(status,0);assert.equal(output.trim(),'True True')
})
