import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, writeFileSync, readFileSync, rmSync, realpathSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { execFileSync, spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./cx-worker.mjs', import.meta.url))
function fixture(t, options = {}) {
  const root = realpathSync(mkdtempSync(path.join(tmpdir(), 'chariox-cx-worker-')))
  t.after(() => rmSync(root, { recursive: true }))
  execFileSync('git', ['init', '-q', root])
  const db = path.join(root, 'sessions.json'), calls = path.join(root, 'calls.jsonl'), ipc = path.join(root, 'ipc.mjs')
  writeFileSync(db, JSON.stringify(options.db ?? {}))
  writeFileSync(ipc, `
    import {appendFileSync} from 'node:fs';
    const options=${JSON.stringify(options)};
    export class LocalIpcClient {
      async send(r) {
        appendFileSync(${JSON.stringify(calls)}, JSON.stringify(r)+'\\n');
        if(r.CreateSession) return options.missingSession ? {} : {SessionCreated:{session:{id:'session-1'}}};
        if(r.SpawnAgent) {if(options.failSpawn) throw new Error('worker offline'); return {AgentSpawned:{agent:{id:'agent-1',remote_execution:{worker_kernel_id:'builder'}}}};}
        if(r.DestroyAgent && options.destroyError) throw Object.assign(new Error(options.destroyError === 'missing' ? 'agent \`agent-1\` was not found' : 'worker offline'), {code: 'kernel_request_failed'});
        if(r.DeleteSession && options.missingSessionOnDrop) throw Object.assign(new Error('session missing'), {code: 'session_not_found'});
        if(r.GetSessionHistoryOutline && options.longFinal) return {SessionHistoryOutline:{agents:[{turns:[{completed_at_ms:Date.now(),lifecycle:'completed',blobs:[{blob_id:'final-blob'}],entries:[{entry_index:1,entry:{kind:'provider_output',merge_key:'earlier',text:'EARLIER'.repeat(500)}}],summary:{entry_index:2,fragment_end:12,total_chars:17015,entry:{kind:'provider_output',merge_key:'final',text:'REMOTE FINAL'}}}]}]}};
        if(r.GetSessionHistoryBlobContent) return {SessionHistoryBlobContent:{entries:[{entry_index:2,entry:{kind:'provider_output',merge_key:'final',text:'x'.repeat(17000)+'LONG_FINAL_TAIL'}}]}};
        if(r.GetSessionHistoryOutline) return {SessionHistoryOutline:{agents:[{turns:[{started_at_ms:Date.now(),completed_at_ms:Date.now(),lifecycle:options.lifecycle??'completed',entries: options.earlierOutput ? [{entry:{kind:'provider_output',text:'EARLIER'}}] : options.inline ? [{entry:{kind:'provider_output',merge_key:options.noMergeKey?undefined:'final',text:'REMOTE FINAL'}}] : [],summary:{entry:{kind:'provider_output',merge_key:options.noMergeKey?undefined:'final',text:'REMOTE FINAL'}}}]}]}};
        if(r.AttachToSession) return {SessionAttached:{attachment:{id:'attachment-1'}}};
        if(r.SubmitPrompt) return {PromptSubmitted:{}};
        if(r.ListAgents) return {AgentsListed:{agents:[{id:'agent-1',state:'Focused',model:'gpt-6.1-sol',is_processing:false}]}};
        return {};
      }
      close(){}
    }
  `)
  const run = (...args) => spawnSync(process.execPath, [script, ...args], { cwd: root, encoding: 'utf8', env: { ...process.env, CX_IPC_MODULE: ipc, CX_SESSION_DB: db, CX_REMOTE_KERNEL: 'builder', CX_PROFILE: 'test-codex-account', KERNEL_URL: 'ws://127.0.0.1:12345/kernel' } })
  const requests = () => { try { return readFileSync(calls, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse) } catch { return [] } }
  return { root, db, run, requests }
}

test('remote spawn keeps the home checkout and sends the worker path and selected account', t => {
  const f = fixture(t), r = f.run('new-remote', 'worker', '/w/repo')
  assert.equal(r.status, 0, r.stderr)
  const req = f.requests()
  assert.equal(req[0].CreateSession.workspace_id, f.root)
  assert.equal(req[0].CreateSession.worktree_id, f.root)
  const spawn = req[1].SpawnAgent
  assert.equal(spawn.worktree_id, '/w/repo')
  assert.equal(spawn.kernel_ref, 'builder')
  assert.equal(spawn.account_profile, 'test-codex-account')
  assert.equal(spawn.model, 'gpt-6.1-sol')
  assert.equal(spawn.effort, 'high')
  assert.equal(JSON.parse(readFileSync(f.db)).worker.kernel, 'builder')
})

test('relative remote paths and reused aliases cause no kernel mutations', t => {
  const f = fixture(t, { db: { existing: { session: 'keep' } } })
  assert.notEqual(f.run('new-remote', 'worker', 'relative').status, 0)
  assert.notEqual(f.run('new-remote', 'existing', '/w/repo').status, 0)
  assert.deepEqual(f.requests(), [])
  assert.equal(JSON.parse(readFileSync(f.db)).existing.session, 'keep')
})

test('failed remote spawn deletes only the new session and does not register an alias', t => {
  const f = fixture(t, { failSpawn: true }), r = f.run('new-remote', 'worker', '/w/repo')
  assert.notEqual(r.status, 0)
  assert.match(r.stderr, /worker offline/)
  assert.equal(f.requests().at(-1).DeleteSession.session_ref, 'session-1')
  assert.deepEqual(JSON.parse(readFileSync(f.db)), {})
})

test('out includes a summary-only remote final exactly once', t => {
  for (const inline of [false, true]) {
    const f = fixture(t, { inline, db: { worker: { session: 'session-1', agent: 'agent-1' } } })
    const r = f.run('out', 'worker')
    assert.equal(r.status, 0, r.stderr)
    assert.equal(r.stdout.match(/REMOTE FINAL/g)?.length, 1)
  }
})

test('say, wait, and st use the home session and remote agent IDs', t => {
  const f = fixture(t, { db: { worker: { session: 'session-1', agent: 'agent-1', kernel: 'builder' } } })
  const prompt = path.join(f.root, 'prompt.txt')
  writeFileSync(prompt, 'smoke')
  for (const args of [['say', 'worker', prompt], ['wait', 'worker', '1'], ['st']]) {
    const r = f.run(...args)
    assert.equal(r.status, 0, r.stderr)
    if (args[0] === 'wait') assert.match(r.stdout, /wait ended: completed/)
  }
  const req = f.requests()
  assert.equal(req.find(r => r.SubmitPrompt).SubmitPrompt.target_agent_id, 'agent-1')
  assert.equal(req.find(r => r.GetSessionHistoryOutline).GetSessionHistoryOutline.session_id, 'session-1')
  assert.equal(req.find(r => r.ListAgents).ListAgents.session_id, 'session-1')
})


test('wait rejects failed and cancelled remote turns', t => {
  for (const lifecycle of ['failed', 'cancelled']) {
    const f = fixture(t, { lifecycle, db: { worker: { session: 'session-1', agent: 'agent-1' } } })
    const r = f.run('wait', 'worker', '1')
    assert.notEqual(r.status, 0)
    assert.match(r.stderr, new RegExp(`turn ${lifecycle}`))
  }
})

test('out retains an unkeyed final after an earlier unkeyed message', t => {
  const f = fixture(t, { noMergeKey: true, earlierOutput: true, db: { worker: { session: 'session-1', agent: 'agent-1' } } })
  const r = f.run('out', 'worker')
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /EARLIER/)
  assert.match(r.stdout, /EARLIER\nREMOTE FINAL/)
})

test('missing session confirmation causes no agent spawn or deletion', t => {
  const f = fixture(t, { missingSession: true }), r = f.run('new-remote', 'worker', '/w/repo')
  assert.notEqual(r.status, 0)
  assert.equal(f.requests().length, 1)
  assert.ok(f.requests()[0].CreateSession)
})

test('drop destroys the leased agent before deleting its home session', t => {
  const f = fixture(t, { db: { worker: { session: 'session-1', agent: 'agent-1' } } })
  const r = f.run('drop', 'worker')
  assert.equal(r.status, 0, r.stderr)
  assert.deepEqual(f.requests().map(r => Object.keys(r)[0]), ['DestroyAgent', 'DeleteSession'])
  assert.deepEqual(JSON.parse(readFileSync(f.db)), {})
})


test('out orders a long final blob after earlier inline commentary', t => {
  const f = fixture(t, { longFinal: true, db: { worker: { session: 'session-1', agent: 'agent-1' } } })
  const r = f.run('out', 'worker')
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /LONG_FINAL_TAIL/)
  assert.doesNotMatch(r.stdout, /EARLIER|REMOTE FINAL/)
})

test('drop can retry after the agent or whole session was already removed', t => {
  for (const missingSessionOnDrop of [false, true]) {
    const f = fixture(t, { destroyError: 'missing', missingSessionOnDrop, db: { worker: { session: 'session-1', agent: 'agent-1' } } })
    const r = f.run('drop', 'worker')
    assert.equal(r.status, 0, r.stderr)
    assert.deepEqual(f.requests().map(r => Object.keys(r)[0]), ['DestroyAgent', 'DeleteSession'])
    assert.deepEqual(JSON.parse(readFileSync(f.db)), {})
  }
})

test('drop preserves the alias and session when worker cleanup fails', t => {
  const f = fixture(t, { destroyError: 'offline', db: { worker: { session: 'session-1', agent: 'agent-1' } } })
  const r = f.run('drop', 'worker')
  assert.notEqual(r.status, 0)
  assert.deepEqual(f.requests().map(r => Object.keys(r)[0]), ['DestroyAgent'])
  assert.equal(JSON.parse(readFileSync(f.db)).worker.session, 'session-1')
})
