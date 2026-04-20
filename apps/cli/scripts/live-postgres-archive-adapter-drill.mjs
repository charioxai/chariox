#!/usr/bin/env node
import { spawn } from 'node:child_process'
import http from 'node:http'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const {
  createSessionRequest,
  attachToSessionRequest,
  spawnAgentRequest,
  launchProviderRunRequest,
  submitPromptRequest,
  completePromptRequest,
  searchHistoryRequest,
} = requests

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false, timeoutMs: 30_000 }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-postgres-archive-adapter-drill.mjs [--keep-artifacts-on-failure] [--timeout-ms MS]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function log(name, details) {
  if (details === undefined) console.log(`[postgres-archive-drill] ${name}`)
  else console.log(`[postgres-archive-drill] ${name}`, JSON.stringify(details))
}

function tomlString(value) {
  return String(value).replaceAll('\\', '\\\\').replaceAll('"', '\\"')
}

function sqlString(value) {
  if (value == null) return 'NULL'
  return `'${String(value).replaceAll("'", "''")}'`
}

function jsonSql(value) {
  return `${sqlString(JSON.stringify(value))}::jsonb`
}

function pushSqlFilter(clauses, field, value) {
  if (value == null || String(value).trim() === '') return
  clauses.push(`${field} = ${sqlString(value)}`)
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
    if (options.input != null) child.stdin.end(options.input)
    else child.stdin.end()
  })
}

async function mustRun(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

function makePorts() {
  const kernelPort = 51000 + Math.floor(Math.random() * 1000)
  const adapterPort = 54000 + Math.floor(Math.random() * 1000)
  return {
    adapterPort,
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

async function buildKernelBinaries() {
  await mustRun('cargo', [
    'build',
    '--manifest-path',
    path.join(repoRoot, 'apps/kernel/Cargo.toml'),
    '--bin',
    'arroba-kernel',
    '--bin',
    'arroba-history-archive-flush',
  ])
  return {
    kernel: path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
    archiveFlush: path.join(repoRoot, 'apps/kernel/target/debug/arroba-history-archive-flush'),
  }
}

function startDaemon(binary, env) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(binary, [], {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  return child
}

async function waitForDaemon(kernelUrl, daemon, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    if (daemon?.exitCode !== null || daemon?.signalCode !== null) {
      throw new Error(`daemon exited before ready code=${daemon.exitCode} signal=${daemon.signalCode}\nstdout:\n${daemon.logs?.stdout ?? ''}\nstderr:\n${daemon.logs?.stderr ?? ''}`)
    }
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send({ GetDaemonHealth: null })
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}\nstdout:\n${daemon?.logs?.stdout ?? ''}\nstderr:\n${daemon?.logs?.stderr ?? ''}`)
}

function variant(response, key) {
  if (!response || !response[key]) {
    throw new Error(`expected ${key}, got ${JSON.stringify(response)}`)
  }
  return response[key]
}

function oneOfVariant(response, keys) {
  for (const key of keys) {
    if (response?.[key]) return response[key]
  }
  throw new Error(`expected one of ${keys.join(', ')}, got ${JSON.stringify(response)}`)
}

async function waitForHistoryMatch(client, query, filters, label, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  let lastEvents = []
  while (Date.now() < deadline) {
    const response = variant(await client.send(searchHistoryRequest(query, filters)), 'HistoryEvents')
    lastEvents = response.events ?? []
    if (lastEvents.length > 0) return lastEvents
    await sleep(250)
  }
  throw new Error(`timed out waiting for history match ${label}; last=${JSON.stringify(lastEvents)}`)
}

async function startPostgres() {
  const name = `arroba-archive-pg-${process.pid}-${Date.now()}`
  const started = await run('docker', [
    'run',
    '--name',
    name,
    '-e',
    'POSTGRES_PASSWORD=arroba',
    '-e',
    'POSTGRES_DB=arroba_history',
    '-d',
    'postgres:16-alpine',
  ])
  if (started.code !== 0) {
    throw new Error(`failed to start postgres:16-alpine with Docker. Start Docker/Colima first, then rerun the drill.\nstdout:\n${started.stdout}\nstderr:\n${started.stderr}`)
  }
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const ready = await run('docker', ['exec', name, 'pg_isready', '-U', 'postgres', '-d', 'arroba_history'])
    if (ready.code === 0) return name
    await sleep(500)
  }
  throw new Error(`Postgres container did not become ready: ${name}`)
}

async function stopPostgres(name) {
  if (name) await run('docker', ['rm', '-f', name]).catch(() => {})
}

async function psql(container, sql, options = {}) {
  const args = ['exec', '-i', container, 'psql', '-U', 'postgres', '-d', 'arroba_history', '-v', 'ON_ERROR_STOP=1']
  if (options.tuplesOnly) args.push('-At')
  return await mustRun('docker', args, { input: sql })
}

async function initPostgres(container) {
  await psql(container, `
CREATE TABLE IF NOT EXISTS archive_events (
  event_id TEXT PRIMARY KEY,
  sequence BIGINT,
  timestamp_ms BIGINT,
  workspace_id TEXT,
  session_id TEXT,
  agent_id TEXT,
  provider TEXT,
  model TEXT,
  kind TEXT,
  content TEXT,
  payload JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
`)
}

async function postgresCount(container) {
  const result = await psql(container, 'SELECT count(*) FROM archive_events;', { tuplesOnly: true })
  return Number(result.stdout.trim())
}

async function postgresEventIds(container) {
  const result = await psql(container, 'SELECT event_id FROM archive_events ORDER BY event_id;', { tuplesOnly: true })
  return result.stdout.split('\n').map((line) => line.trim()).filter(Boolean)
}

function startAdapter({ container, port, token }) {
  const state = {
    failNextAppend: false,
    rejectNextAppend: false,
    searchEnabled: false,
    appendRequests: 0,
    searchRequests: 0,
    rejectedEventIds: [],
  }
  const server = http.createServer(async (request, response) => {
    try {
      if (request.headers.authorization !== `Bearer ${token}`) {
        response.writeHead(401, { 'content-type': 'application/json' })
        response.end(JSON.stringify({ error: 'missing or invalid bearer token' }))
        return
      }
      if (request.method === 'GET' && request.url === '/arroba/history/capabilities') {
        response.writeHead(200, { 'content-type': 'application/json' })
        response.end(JSON.stringify({
          append: true,
          query: false,
          search: state.searchEnabled,
          full_text_search: state.searchEnabled,
          blob_refs: false,
        }))
        return
      }
      if (request.method === 'POST' && request.url === '/arroba/history/search') {
        state.searchRequests += 1
        if (!state.searchEnabled) {
          response.writeHead(404, { 'content-type': 'application/json' })
          response.end(JSON.stringify({ error: 'archive search disabled' }))
          return
        }
        let body = ''
        for await (const chunk of request) body += chunk.toString()
        const payload = JSON.parse(body)
        const query = payload.query ?? {}
        const clauses = ['1 = 1']
        pushSqlFilter(clauses, 'session_id', query.session_id)
        pushSqlFilter(clauses, 'agent_id', query.agent_id)
        pushSqlFilter(clauses, 'provider', query.provider)
        pushSqlFilter(clauses, 'model', query.model)
        pushSqlFilter(clauses, "payload->>'workflow_id'", query.workflow_id)
        pushSqlFilter(clauses, "payload->>'machine_id'", query.machine_id)
        pushSqlFilter(clauses, "payload->>'repo_root'", query.repo_root)
        pushSqlFilter(clauses, "payload->>'worktree_path'", query.worktree_path)
        pushSqlFilter(clauses, 'kind', query.kind)
        if (query.after_sequence != null) clauses.push(`sequence > ${Number(query.after_sequence)}`)
        if (query.text != null && String(query.text).trim() !== '') {
          clauses.push(`(content ILIKE ${sqlString(`%${query.text}%`)} OR payload::text ILIKE ${sqlString(`%${query.text}%`)})`)
        }
        const limit = Math.max(1, Math.min(500, Number(query.limit ?? 100)))
        const result = await psql(container, `
SELECT payload::text
FROM archive_events
WHERE ${clauses.join(' AND ')}
ORDER BY sequence ASC, event_id ASC
LIMIT ${limit};
`, { tuplesOnly: true })
        const events = result.stdout
          .split('\n')
          .map((line) => line.trim())
          .filter(Boolean)
          .map((line) => JSON.parse(line))
        response.writeHead(200, { 'content-type': 'application/json' })
        response.end(JSON.stringify({
          events,
          next_sequence: events.length === limit ? events[events.length - 1].sequence : null,
        }))
        return
      }
      if (request.method !== 'POST' || request.url !== '/arroba/history/events') {
        response.writeHead(404, { 'content-type': 'application/json' })
        response.end(JSON.stringify({ error: 'not found' }))
        return
      }
      state.appendRequests += 1
      let body = ''
      for await (const chunk of request) body += chunk.toString()
      const payload = JSON.parse(body)
      const events = Array.isArray(payload.events) ? payload.events : []
      if (state.failNextAppend) {
        state.failNextAppend = false
        response.writeHead(503, { 'content-type': 'application/json' })
        response.end(JSON.stringify({ error: 'intentional adapter outage' }))
        return
      }
      const accepted = []
      const rejected = []
      let rejectedOne = false
      for (const event of events) {
        if (state.rejectNextAppend && !rejectedOne) {
          rejectedOne = true
          state.rejectedEventIds.push(event.event_id)
          rejected.push({ event_id: event.event_id, reason: 'intentional drill rejection' })
          continue
        }
        await psql(container, `
INSERT INTO archive_events (
  event_id, sequence, timestamp_ms, workspace_id, session_id, agent_id,
  provider, model, kind, content, payload
) VALUES (
  ${sqlString(event.event_id)},
  ${Number(event.sequence ?? 0)},
  ${Number(event.timestamp_ms ?? 0)},
  ${sqlString(event.workspace_id)},
  ${sqlString(event.session_id)},
  ${sqlString(event.agent_id)},
  ${sqlString(event.provider)},
  ${sqlString(event.model)},
  ${sqlString(event.kind)},
  ${sqlString(event.content)},
  ${jsonSql(event)}
)
ON CONFLICT (event_id) DO NOTHING;
`)
        accepted.push(event.event_id)
      }
      state.rejectNextAppend = false
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ accepted_event_ids: accepted, rejected_events: rejected }))
    } catch (error) {
      response.writeHead(500, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ error: error.stack ?? error.message }))
    }
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', () => {
      server.off('error', reject)
      resolve({
        state,
        close: () => new Promise((closeResolve) => server.close(closeResolve)),
      })
    })
  })
}

async function sqliteScalar(dbPath, sql) {
  const result = await mustRun('sqlite3', [dbPath, sql])
  return Number(result.stdout.trim())
}

async function sqliteTextRows(dbPath, sql) {
  const result = await mustRun('sqlite3', ['-noheader', dbPath, sql])
  return result.stdout.split('\n').map((line) => line.trim()).filter(Boolean)
}

async function sqliteJsonRows(dbPath, sql) {
  const result = await mustRun('sqlite3', ['-json', dbPath, sql])
  return JSON.parse(result.stdout.trim() || '[]')
}

async function sqliteExec(dbPath, sql) {
  await mustRun('sqlite3', [dbPath, sql])
}

async function writeConfig({ configHome, historyPath, statePath, adapterUrl, requireDurableAcceptance }) {
  await writeFile(
    path.join(configHome, 'arroba', 'config.toml'),
    `version = 1

[history.operational]
path = "${tomlString(historyPath)}"

[history.archive]
mode = "external"
url = "${tomlString(adapterUrl)}"
token_env = "ARROBA_ARCHIVE_DRILL_TOKEN"
archive_deleted_agents = true
archive_before_delete = true
delete_operational_after_verified_archive = true
require_durable_acceptance = ${requireDurableAcceptance ? 'true' : 'false'}

[state]
path = "${tomlString(statePath)}"
snapshot_interval_events = 1
`,
    'utf8',
  )
}

async function runFlush(binary, env, limit = 100) {
  return await run(binary, ['--limit', String(limit)], { env })
}

async function submitDrillPrompt({ client, session, attachment, agent, marker }) {
  const prompt = `Echo ${marker} in a provider response for the Postgres archive adapter drill.`
  const submitted = variant(await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, [])), 'PromptSubmitted')
  const promptId = submitted.outcome.Started?.prompt?.id ?? submitted.outcome.Started?.prompt_id ?? null
  if (!promptId) throw new Error(`expected started prompt id, got ${JSON.stringify(submitted.outcome)}`)
  await waitForHistoryMatch(client, marker, {
    session_id: session.id,
    kind: 'provider_output',
    provider: 'dev-stub',
    model: 'archive-drill-model',
    limit: 10,
  }, marker)
  await client.send(completePromptRequest(session.id))
  return promptId
}

async function assertPending(dbPath, expected, label) {
  const pending = await sqliteScalar(dbPath, 'SELECT count(*) FROM history_archive_outbox WHERE archived_at_ms IS NULL;')
  if (pending !== expected) throw new Error(`${label}: expected ${expected} pending archive events, got ${pending}`)
}

async function waitForPendingAtLeast(dbPath, minimum, label, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  let pending = 0
  while (Date.now() < deadline) {
    pending = await sqliteScalar(dbPath, 'SELECT count(*) FROM history_archive_outbox WHERE archived_at_ms IS NULL;')
    if (pending >= minimum) return pending
    await sleep(250)
  }
  throw new Error(`${label}: expected at least ${minimum} pending archive events, got ${pending}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const ports = makePorts()
  const root = path.join(repoRoot, 'target', 'live-postgres-archive-adapter-drill', `${process.pid}-${Date.now()}`)
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state-home')
  const workspace = path.join(root, 'workspace')
  const statePath = path.join(root, 'state.db')
  const historyPath = path.join(root, 'history.db')
  const token = `archive-token-${process.pid}-${Date.now()}`
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  let container = null
  let adapter = null
  let daemon = null
  let client = null

  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })

    container = await startPostgres()
    await initPostgres(container)
    adapter = await startAdapter({
      container,
      port: ports.adapterPort,
      token,
    })
    const adapterUrl = `http://127.0.0.1:${ports.adapterPort}`
    await writeConfig({
      configHome,
      historyPath,
      statePath,
      adapterUrl,
      requireDurableAcceptance: true,
    })

    const capsResponse = await fetch(`${adapterUrl}/arroba/history/capabilities`, {
      headers: { authorization: `Bearer ${token}` },
    })
    const caps = await capsResponse.json()
    if (!caps.append || caps.search !== false) throw new Error(`unexpected adapter capabilities: ${JSON.stringify(caps)}`)

    const binaries = await buildKernelBinaries()
    const env = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: configHome,
      XDG_STATE_HOME: stateHome,
      XDG_DATA_HOME: path.join(home, '.local/share'),
      XDG_CACHE_HOME: path.join(home, '.cache'),
      ARROBA_ARCHIVE_DRILL_TOKEN: token,
      ARROBA_DAEMON_ID: `postgres-archive-${process.pid}`,
      ARROBA_MACHINE_ID: `machine-postgres-archive-${process.pid}`,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
      ARROBA_SESSION_HISTORY_DIR: path.join(root, 'session-history'),
      ARROBA_PROVIDER_DEV_STUB: '1',
    }

    daemon = startDaemon(binaries.kernel, env)
    await waitForDaemon(kernelUrl, daemon, options.timeoutMs)
    client = new LocalIpcClient(kernelUrl)

    const session = variant(await client.send(createSessionRequest(workspace, workspace, 'postgres-archive-drill')), 'SessionCreated').session
    const attachment = variant(await client.send(attachToSessionRequest(session.id, `postgres-archive-drill-${process.pid}`)), 'SessionAttached').attachment
    const agent = variant(
      await client.send(spawnAgentRequest(session.id, 'dev-stub', 'archive-agent', 'archive-drill-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    const launched = oneOfVariant(
      await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'archive-drill-model', 'low', agent.id)),
      ['ProviderRunLaunchAccepted', 'ProviderRunLaunched'],
    )
    log('provider-launched', {
      sessionId: session.id,
      agentId: agent.id,
      providerRunId: launched.provider_run?.id ?? null,
    })

    const markerA = `POSTGRES_ARCHIVE_A_${process.pid}_${Date.now()}`
    await submitDrillPrompt({ client, session, attachment, agent, marker: markerA })
    const markerA2 = `POSTGRES_ARCHIVE_A2_${process.pid}_${Date.now()}`
    await submitDrillPrompt({ client, session, attachment, agent, marker: markerA2 })
    const initialPending = await waitForPendingAtLeast(historyPath, 2, 'initial outbox enqueue')
    const initialEventJson = await sqliteTextRows(historyPath, 'SELECT event_json FROM history_archive_outbox ORDER BY created_at_ms, event_id LIMIT 1;')
    if (initialEventJson.length !== 1) throw new Error('expected one event_json row for idempotency check')
    log('outbox-enqueued', { initialPending })

    adapter.state.failNextAppend = true
    const failedFlush = await runFlush(binaries.archiveFlush, env)
    if (failedFlush.code === 0) throw new Error(`expected failed flush to exit nonzero, got stdout=${failedFlush.stdout}`)
    await assertPending(historyPath, initialPending, 'after HTTP failure')
    const failedRows = await sqliteJsonRows(historyPath, 'SELECT attempts, last_error FROM history_archive_outbox WHERE archived_at_ms IS NULL;')
    if (!failedRows.every((row) => Number(row.attempts) >= 1 && String(row.last_error ?? '').includes('HTTP 503'))) {
      throw new Error(`expected failed rows to record HTTP 503, got ${JSON.stringify(failedRows)}`)
    }
    if (await postgresCount(container) !== 0) throw new Error('HTTP failure should not insert Postgres archive rows')
    log('http-failure-retained-pending', { pending: initialPending })

    adapter.state.rejectNextAppend = true
    const rejectedDurableFlush = await runFlush(binaries.archiveFlush, env)
    if (rejectedDurableFlush.code === 0) throw new Error(`expected durable partial rejection to exit nonzero, got stdout=${rejectedDurableFlush.stdout}`)
    await assertPending(historyPath, initialPending, 'after durable partial rejection')
    const partialPostgresCount = await postgresCount(container)
    if (partialPostgresCount !== initialPending - 1) {
      throw new Error(`expected partial Postgres insert count ${initialPending - 1}, got ${partialPostgresCount}`)
    }
    log('durable-partial-rejection-retained-pending', { pending: initialPending, partialPostgresCount })

    const successFlush = await runFlush(binaries.archiveFlush, env)
    if (successFlush.code !== 0) throw new Error(`expected retry flush to succeed\nstdout=${successFlush.stdout}\nstderr=${successFlush.stderr}`)
    await assertPending(historyPath, 0, 'after successful retry')
    const archivedCount = await sqliteScalar(historyPath, 'SELECT count(*) FROM history_archive_outbox WHERE archived_at_ms IS NOT NULL;')
    if (archivedCount !== initialPending) throw new Error(`expected ${initialPending} archived rows, got ${archivedCount}`)
    if (await postgresCount(container) !== initialPending) throw new Error('successful retry should archive all initial events exactly once')
    log('retry-accepted', { archivedCount })

    const duplicateEvent = JSON.parse(initialEventJson[0])
    for (let index = 0; index < 2; index += 1) {
      const response = await fetch(`${adapterUrl}/arroba/history/events`, {
        method: 'POST',
        headers: {
          authorization: `Bearer ${token}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify({ events: [duplicateEvent] }),
      })
      if (!response.ok) throw new Error(`direct idempotency append failed with ${response.status}: ${await response.text()}`)
      const body = await response.json()
      if (!body.accepted_event_ids?.includes(duplicateEvent.event_id)) throw new Error(`duplicate append was not accepted: ${JSON.stringify(body)}`)
    }
    if (await postgresCount(container) !== initialPending) throw new Error('duplicate adapter appends should not create extra Postgres rows')
    log('adapter-idempotency-ok', { eventId: duplicateEvent.event_id })

    await writeConfig({
      configHome,
      historyPath,
      statePath,
      adapterUrl,
      requireDurableAcceptance: false,
    })
    const markerB = `POSTGRES_ARCHIVE_B_${process.pid}_${Date.now()}`
    await submitDrillPrompt({ client, session, attachment, agent, marker: markerB })
    const markerB2 = `POSTGRES_ARCHIVE_B2_${process.pid}_${Date.now()}`
    await submitDrillPrompt({ client, session, attachment, agent, marker: markerB2 })
    const secondPending = await waitForPendingAtLeast(historyPath, 2, 'second outbox enqueue')
    adapter.state.rejectNextAppend = true
    const rejectedNonDurableFlush = await runFlush(binaries.archiveFlush, env)
    if (rejectedNonDurableFlush.code !== 0) {
      throw new Error(`expected non-durable rejection flush to succeed with rejected outcome\nstdout=${rejectedNonDurableFlush.stdout}\nstderr=${rejectedNonDurableFlush.stderr}`)
    }
    const rejectedOutcome = JSON.parse(rejectedNonDurableFlush.stdout.trim())
    if (rejectedOutcome.rejected_events.length !== 1) throw new Error(`expected one rejected event, got ${rejectedNonDurableFlush.stdout}`)
    await assertPending(historyPath, 1, 'after non-durable rejection')
    const rejectedPending = await sqliteJsonRows(historyPath, 'SELECT attempts, last_error FROM history_archive_outbox WHERE archived_at_ms IS NULL;')
    if (rejectedPending.length !== 1 || Number(rejectedPending[0].attempts) < 1 || !String(rejectedPending[0].last_error ?? '').includes('rejected')) {
      throw new Error(`expected rejected row to remain retryable, got ${JSON.stringify(rejectedPending)}`)
    }
    log('non-durable-rejection-partial-checkpoint-ok', { rejectedEventId: rejectedOutcome.rejected_events[0].event_id })

    const finalFlush = await runFlush(binaries.archiveFlush, env)
    if (finalFlush.code !== 0) throw new Error(`expected final rejected-row retry to succeed\nstdout=${finalFlush.stdout}\nstderr=${finalFlush.stderr}`)
    await assertPending(historyPath, 0, 'after final rejected-row retry')
    const ids = await postgresEventIds(container)
    if (new Set(ids).size !== ids.length) throw new Error(`Postgres archive has duplicate event ids: ${JSON.stringify(ids)}`)

    const historyAfterArchive = variant(await client.send(searchHistoryRequest(markerA, {
      session_id: session.id,
      provider: 'dev-stub',
      model: 'archive-drill-model',
      limit: 10,
    })), 'HistoryEvents').events
    if (historyAfterArchive.length === 0) throw new Error('operational history search should still work with externally managed archive search disabled')

    await sqliteExec(
      historyPath,
      `DELETE FROM history_events WHERE session_id = ${sqlString(session.id)} AND (content LIKE ${sqlString(`%${markerA}%`)} OR metadata_text LIKE ${sqlString(`%${markerA}%`)});`,
    )
    const operationalOnlyAfterDelete = variant(await client.send(searchHistoryRequest(markerA, {
      session_id: session.id,
      provider: 'dev-stub',
      model: 'archive-drill-model',
      limit: 10,
    })), 'HistoryEvents').events
    if (operationalOnlyAfterDelete.length !== 0) {
      throw new Error(`expected operational-only search to miss deleted local event, got ${JSON.stringify(operationalOnlyAfterDelete)}`)
    }
    adapter.state.searchEnabled = true
    const archiveSearch = variant(await client.send(searchHistoryRequest(markerA, {
      session_id: session.id,
      provider: 'dev-stub',
      model: 'archive-drill-model',
      limit: 10,
    })), 'HistoryEvents').events
    if (archiveSearch.length === 0 || !archiveSearch.some((event) => String(event.content ?? '').includes(markerA))) {
      throw new Error(`expected Arroba search to return deleted local event from Postgres archive, got ${JSON.stringify(archiveSearch)}`)
    }

    log('passed', {
      archivedEvents: ids.length,
      adapterAppendRequests: adapter.state.appendRequests,
      adapterSearchRequests: adapter.state.searchRequests,
      searchMode: 'operational-and-postgres-archive',
    })
  } finally {
    if (client) await client.close().catch(() => {})
    if (daemon && daemon.exitCode === null && daemon.signalCode === null) daemon.kill('SIGTERM')
    if (adapter) await adapter.close().catch(() => {})
    await stopPostgres(container)
    if (!options.keepArtifactsOnFailure) {
      await rm(root, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`[postgres-archive-drill] kept drill artifacts at ${root}`)
    }
  }
}

main().catch((error) => {
  console.error(`[postgres-archive-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
