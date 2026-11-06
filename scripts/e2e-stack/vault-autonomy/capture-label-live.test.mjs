// MP-08 / MP-09 / MP-10 / MP-11: failed live drills must leave no polling or agent work.
import assert from 'node:assert/strict'
import test from 'node:test'
import {mkdtemp, mkdir, writeFile, rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import path from 'node:path'
import {createRequire} from 'node:module'
import {execFile} from 'node:child_process'
import {promisify} from 'node:util'
const wsFrom = new URL('../../../apps/cli/package.json', import.meta.url)
const {WebSocketServer} = createRequire(wsFrom)('ws')

test('a failed agent prompt stops polling, ends its session and exits', async () => {
  const scratch = await mkdtemp(path.join(tmpdir(), 'vflag-label-cleanup-'))
  const server = new WebSocketServer({host: '127.0.0.1', port: 0})
  await new Promise(resolve => server.once('listening', resolve))
  const calls = []
  server.on('connection', socket => socket.on('message', data => {
    const frame = JSON.parse(String(data)), kind = Object.keys(frame.request)[0]
    calls.push(kind)
    const response = {
      LinkProviderAccountProfile: {ProviderAccountProfile: {profile: {profile_id: 'profile'}}},
      ListWorkspaceWorktrees: {WorkspaceWorktreesListed: {worktrees: [{path: scratch}]}},
      CreateSession: {SessionCreated: {session: {id: 'drill-session'}, agent: {id: 'drill-agent'}}},
      AttachToSession: {SessionAttached: {attachment: {id: 'attachment'}}},
      PumpTerminalOutput: {TerminalOutput: {records: []}},
      ListSessions: {SessionsListed: {sessions: [{id: 'drill-session', active_interactions: []}]}},
      EndSession: {SessionEnded: {}},
    }[kind]
    socket.send(JSON.stringify({request_id: frame.request_id, ...(kind === 'SubmitPrompt'
      ? {error: {message: 'forced prompt failure'}} : {response})}))
  }))
  try {
    await mkdir(path.join(scratch, 'home/state/kernel-local-auth'), {recursive: true})
    await writeFile(path.join(scratch, 'ports.json'), JSON.stringify({kernel: server.address().port}))
    await writeFile(path.join(scratch, `home/state/kernel-local-auth/${server.address().port}.token`), 'fixture-auth')
    const passkey = path.join(scratch, 'passkey')
    await writeFile(passkey, 'fixture-passphrase')
    const result = await promisify(execFile)(process.execPath, [new URL('./capture-label-live.mjs', import.meta.url).pathname], {
      timeout: 7000, env: {...process.env, ROOM_KERNEL_ROOT: scratch, ROOM_WS_FROM: wsFrom.pathname,
        R6_WORKSPACE: scratch, R6_PROFILE: scratch, R6_PASSKEY_FILE: passkey, R6_EVIDENCE: scratch},
    })
    assert.match(result.stdout, /forced prompt failure/)
    assert.ok(calls.includes('EndSession'), 'the drill-created session must be ended')
    assert.equal(calls.at(-1), 'EndSession', 'polling must stop before the session ends')
  } finally {
    for (const socket of server.clients) socket.terminate()
    await new Promise(resolve => server.close(resolve))
    await rm(scratch, {recursive: true, force: true})
  }
})
