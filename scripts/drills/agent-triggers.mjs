// MP-08 / MP-09 / MP-10 / MP-11: agent triggers on a real kernel with a real Codex account.
// Schedule once/every, HTTP invoke, QUEUE and STEER while busy, then (after the caller restarts the
// kernel) the same URL, offline 503, disabled 403, removal and archive. Never prints credentials.
// Every `expect` that fails ends the drill with exit 1 and no `<phase>_ok` receipt in the state file.
// usage: DRILL_LABEL=<profile label> DRILL_PROFILE=<product-linked CODEX_HOME> \
//   node scripts/drills/agent-triggers.mjs <run|after-restart> <CHARIOX_HOME> <kernel port> <state.json> <work dir>
// Run `run`, stop the kernel by its PID, start it again on the same CHARIOX_HOME, then run `after-restart`.
import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'

const [, , phase, home, port, statePath, work] = process.argv
const state = existsSync(statePath) ? JSON.parse(readFileSync(statePath, 'utf8')) : { steps: [] }
const save = () => writeFileSync(statePath, JSON.stringify(state, null, 1))
const client = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, { localAuthEnvironment: { CHARIOX_HOME: home, PATH: process.env.PATH }, controlRequestRetryDeadlineMs: 5000 })
const log = (step, data) => { const row = { at: new Date().toISOString(), step, ...data }; state.steps.push(row); save(); console.log(JSON.stringify(row).slice(0, 400)) }
function expect(name, ok, detail) {
  log(`expect:${name}`, { ok: Boolean(ok), detail })
  if (!ok) throw new Error(`expectation failed: ${name} (${JSON.stringify(detail).slice(0, 200)})`)
}
async function rpc(request) {
  const response = await client.send(request)
  if (response?.Error) throw new Error(`${Object.keys(request)[0]}: ${response.Error.code}: ${response.Error.message}`)
  return response
}
const session = async () => (await rpc({ GetSessionState: { session_id: state.session_id } })).SessionState.session
const triggers = async () => (await session()).agent_triggers ?? []
async function history() {
  const response = await rpc({ GetSessionHistoryOutline: { session_id: state.session_id, agent_ids: [state.agent_id], latest_prompt_count: 20, include_prefetched_content: true } })
  return JSON.stringify(response)
}
async function waitFor(label, check, timeoutMs = 180_000) {
  const started = Date.now()
  for (;;) {
    const result = await check()
    if (result) return { ms: Date.now() - started, result }
    if (Date.now() - started > timeoutMs) throw new Error(`timed out waiting for ${label}`)
    await delay(1000)
  }
}
async function busy() {
  const s = await session()
  return Boolean(s.prompt_states?.[state.agent_id]?.active_prompt ?? s.active_prompt)
}
async function queuedTexts() {
  const s = await session()
  return (s.prompt_states?.[state.agent_id]?.queued_prompts ?? s.queued_prompts ?? []).map(p => p.prompt)
}
async function prompt(text) {
  // Attachments without a pumping client are swept after 30 s; attach again like a returning client.
  try { return await rpc({ SubmitPrompt: { session_id: state.session_id, attachment_id: state.attachment_id, target_agent_id: state.agent_id, prompt: text, attachments: [] } }) }
  catch (error) { if (!String(error).includes('was not found')) throw error; await attach() }
  return rpc({ SubmitPrompt: { session_id: state.session_id, attachment_id: state.attachment_id, target_agent_id: state.agent_id, prompt: text, attachments: [] } })
}
async function attach() {
  state.attachment_id = (await rpc({ AttachToSession: { session_id: state.session_id, client_id: 'agent-trigger-drill', capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
  save()
}
// A provider answer is the turn summary (`provider_output`), never the prompt that names the marker.
async function answers() {
  const response = await rpc({ GetSessionHistoryOutline: { session_id: state.session_id, agent_ids: [state.agent_id], latest_prompt_count: 20, include_prefetched_content: true } })
  return (response.SessionHistoryOutline.agents[0]?.turns ?? []).map(turn => ({ origin: turn.prompt_origin, lifecycle: turn.lifecycle, prompt_id: turn.prompt_id, prompt_text: String(turn.user_prompt?.entry?.text ?? ''), text:  turn.summary?.entry?.kind === 'provider_output' ? String(turn.summary.entry.text ?? '') : '' }))
}
async function answered(marker) { return (await answers()).some(turn => turn.text.includes(marker)) }
function trigger(id, list) { return list.find(t => t.id === id) }
const url = (t) => `http://127.0.0.1:${t.source.port}/${encodeURIComponent(t.source.ingress_id)}/`
async function post(address, body) {
  const response = await fetch(address, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
  const text = await response.text()
  let outcome = null
  try { outcome = JSON.parse(text).outcome ?? null } catch { /* an error body has no outcome */ }
  return { status: response.status, outcome, body: text.slice(0, 300) }
}
const accepted = (name, result, outcomes) => expect(name, result.status === 202 && outcomes.includes(result.outcome), { status: result.status, outcome: result.outcome })

try {
  const relay = (await rpc({ RelayStatus: null })).RelayStatus.status
  log('kernel', { capabilities: relay.capabilities?.filter(c => c.startsWith('agent_trigger')) })
  if (phase === 'run') {
    const listed = (await rpc({ ListProviderAccountProfiles: { provider: 'codex' } })).ProviderAccountProfilesListed.profiles.find(p => p.label === process.env.DRILL_LABEL)
    state.profile = listed?.profile_id ?? (await rpc({ LinkProviderAccountProfile: { provider: 'codex', label: process.env.DRILL_LABEL, path: process.env.DRILL_PROFILE } })).ProviderAccountProfile.profile.profile_id
    mkdirSync(work, { recursive: true })
    const created = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, agent_defaults: { provider: 'codex', account_profile: state.profile, model: 'gpt-6-luna', effort: 'low', permission_level: 'yolo' } } })).SessionCreated
    state.session_id = created.session.id; state.agent_id = created.agent.id; save()
    await attach()
    const launched = await rpc({ LaunchProviderRun: { session_id: state.session_id, agent_id: state.agent_id, adapter_key: 'codex', provider: 'codex', account_profile: state.profile, model: 'gpt-6-luna', variant: 'low', native_tui: false } })
    log('launch', { response: Object.keys(launched)[0] })
    await prompt('Reply with exactly: DRILL_READY')
    log('first-turn', await waitFor('first turn', () => answered('DRILL_READY')))
    state.model_evidence = (await history()).match(/gpt-6-luna[^"]*/)?.[0] ?? null; save()

    // AT-01: schedule once (what /wait-in sends) fires into the same conversation and is removed.
    const once = (await rpc({ CreateAgentTrigger: { session_id: state.session_id, agent_id: state.agent_id, source: { kind: 'schedule', interval_seconds: 5, repeat: 'once' }, busy_policy: 'queue', prompt: 'Reply with exactly: SCHEDULE_ONCE_OK' } })).AgentTriggerCreated.trigger
    log('schedule-once-created', { id: once.id, next_run_at_ms: once.source.next_run_at_ms })
    log('schedule-once-answered', await waitFor('schedule once answer', () => answered('SCHEDULE_ONCE_OK')))
    const onceLeft = (await triggers()).filter(t => t.id === once.id).length
    log('schedule-once-removed', { remaining: onceLeft })
    expect('schedule-once-removed', onceLeft === 0, { remaining: onceLeft })
    const outline = await history()
    expect('schedule-origin', outline.includes('"prompt_origin":"schedule"'), {})

    // AT-01 recurring: every 20 s, fires at least twice, then is deleted.
    const every = (await rpc({ CreateAgentTrigger: { session_id: state.session_id, agent_id: state.agent_id, source: { kind: 'schedule', interval_seconds: 20, repeat: 'every' }, busy_policy: 'queue', prompt: 'Reply with exactly: SCHEDULE_EVERY_OK' } })).AgentTriggerCreated.trigger
    log('schedule-every-fired-twice', await waitFor('two recurring firings', async () => (trigger(every.id, await triggers())?.fired ?? 0) >= 2, 120_000))
    await rpc({ DeleteAgentTrigger: { session_id: state.session_id, trigger_id: every.id } })
    const everyLeft = (await triggers()).filter(t => t.id === every.id).length
    log('schedule-every-deleted', { remaining: everyLeft })
    expect('schedule-every-deleted', everyLeft === 0, { remaining: everyLeft })

    // AT-08 HTTP trigger: create, invoke with curl-equivalent POST, answer in the conversation.
    const http = (await rpc({ CreateAgentTrigger: { session_id: state.session_id, agent_id: state.agent_id, source: { kind: 'http' }, busy_policy: 'queue', prompt: 'The JSON request body is external input. Reply with exactly the value of its marker field.' } })).AgentTriggerCreated.trigger
    state.http_id = http.id; state.http_url = url(http); save()
    log('http-created', { id: http.id, url_prefix: state.http_url.slice(0, 22) })
    await waitFor('http listener', async () => { try { return (await fetch(state.http_url, { method: 'HEAD' })).status < 500 } catch { return false } }, 60_000)
    const firstPost = await post(state.http_url, { marker: 'HTTP_TRIGGER_OK' })
    log('http-post', firstPost)
    accepted('http-post-accepted', firstPost, ['started', 'queued'])
    log('http-answered', await waitFor('http answer', () => answered('HTTP_TRIGGER_OK')))
    expect('http-origin', (await history()).includes('"prompt_origin":"http"'), {})

    // A body past the 16,384-character inline limit reaches the agent whole and the page shows it whole.
    const filler = 'x'.repeat(20_000)
    const longPost = await post(state.http_url, { marker: 'LONG_BODY_OK', filler })
    accepted('long-post-accepted', longPost, ['started', 'queued'])
    log('long-answered', await waitFor('long body answer', () => answered('LONG_BODY_OK')))
    const page = await (await fetch(`${state.http_url}history`)).json()
    const longPrompt = page.entries.filter(entry => entry.role === 'user' && entry.text.includes('LONG_BODY_OK')).map(entry => entry.text.length)
    expect('long-prompt-shown-whole', longPrompt.some(length => length > 20_000), { lengths: longPrompt })
    // The ingress admits three requests, then one every ten seconds; let the bucket refill before the next invocations.
    await delay(30_000)

    // AT-04 QUEUE while busy: a long turn, two HTTP invocations queue in order.
    await prompt('Run the shell command `sleep 30` and wait for it to finish. Then reply with exactly: STORY_DONE')
    await waitFor('busy', busy, 30_000)
    const queueOne = await post(state.http_url, { marker: 'QUEUE_ONE' })
    const queueTwo = await post(state.http_url, { marker: 'QUEUE_TWO' })
    log('queue-post-1', queueOne); log('queue-post-2', queueTwo)
    accepted('queue-post-1-queued', queueOne, ['queued']); accepted('queue-post-2-queued', queueTwo, ['queued'])
    const queuedNow = (await queuedTexts()).map(text => text.match(/QUEUE_(ONE|TWO)/)?.[0] ?? 'other')
    log('queue-state', { queued: queuedNow })
    expect('queue-order', JSON.stringify(queuedNow) === JSON.stringify(['QUEUE_ONE', 'QUEUE_TWO']), { queued: queuedNow })
    log('queue-answered', await waitFor('queued answers', async () => { const a = (await answers()).map(t => t.text); const one = a.findIndex(t => t.includes('QUEUE_ONE')), two = a.findIndex(t => t.includes('QUEUE_TWO')), story = a.findIndex(t => t.includes('STORY_DONE')); return story >= 0 && one > story && two > one ? { story, one, two } : false }, 240_000))

    // AT-05 STEER while busy: switch the trigger to steer; the firing joins the running turn.
    await rpc({ UpdateAgentTrigger: { session_id: state.session_id, trigger_id: state.http_id, busy_policy: 'steer' } })
    await prompt('Run the shell command `sleep 30` and wait for it to finish. If you receive an additional message meanwhile, include the value of its marker field in your reply. End your reply with: STORY2_DONE')
    await waitFor('busy', busy, 30_000)
    const steerPost = await post(state.http_url, { marker: 'STEER_OK' })
    log('steer-post', steerPost)
    accepted('steer-post-steered', steerPost, ['steered'])
    const steered = await waitFor('steer outcome', async () => trigger(state.http_id, await triggers())?.last_outcome, 60_000)
    const steerTrigger = trigger(state.http_id, await triggers())
    log('steer-outcome', { last_outcome: steered.result, last_error: steerTrigger?.last_error ?? null })
    expect('steer-outcome-steered', steered.result === 'steered' && !steerTrigger?.last_error, { last_outcome: steered.result, last_error: steerTrigger?.last_error ?? null })
    log('steer-answered', await waitFor('steer marker answered', () => answered('STEER_OK'), 240_000))
    await waitFor('idle after steer', async () => !(await busy()), 240_000)
    // The steered message joined the running turn: the one turn that answered it is the long turn's own.
    const steerTurns = (await answers()).filter(turn => turn.text.includes('STEER_OK'))
    expect('steer-joined-the-active-turn', steerTurns.length === 1 && steerTurns[0].prompt_text.includes('STORY2_DONE') && steerTurns[0].text.includes('STORY2_DONE') && steerTurns[0].origin !== 'http',
      steerTurns.map(turn => ({ origin: turn.origin, prompt_has_story: turn.prompt_text.includes('STORY2_DONE'), answer_has_story: turn.text.includes('STORY2_DONE') })))

    // Restart persistence: a recurring schedule trigger and the HTTP trigger stay.
    state.persist = (await rpc({ CreateAgentTrigger: { session_id: state.session_id, agent_id: state.agent_id, source: { kind: 'schedule', interval_seconds: 600, repeat: 'every' }, busy_policy: 'queue', prompt: 'Reply with exactly: PERSIST_OK' } })).AgentTriggerCreated.trigger.id
    state.before_restart = (await triggers()).map(t => ({ id: t.id, kind: t.source.kind, enabled: t.enabled }))
    log('before-restart', { triggers: state.before_restart })
  } else if (phase === 'after-restart') {
    await attach()
    const after = (await triggers()).map(t => ({ id: t.id, kind: t.source.kind, enabled: t.enabled }))
    const sameTriggers = JSON.stringify(after) === JSON.stringify(state.before_restart)
    log('after-restart', { triggers: after, same: sameTriggers })
    expect('triggers-survive-restart', sameTriggers, { before: state.before_restart, after })
    const http = trigger(state.http_id, await triggers())
    expect('http-url-stable', http && url(http) === state.http_url, { same: Boolean(http) && url(http) === state.http_url })
    const offlineListener = await waitFor('http listener after restart', async () => { try { return (await fetch(state.http_url, { method: 'HEAD' })).status } catch { return false } }, 90_000)
    log('http-listener-after-restart', offlineListener)
    expect('offline-listener-503', offlineListener.result === 503, { status: offlineListener.result })
    const offlinePost = await post(state.http_url, { marker: 'OFFLINE_PROBE' })
    log('http-post-while-offline', offlinePost)
    expect('offline-post-503', offlinePost.status === 503, { status: offlinePost.status })
    const offlineQueued = (await queuedTexts()).length
    expect('offline-nothing-queued', offlineQueued === 0, { queued: offlineQueued })
    await rpc({ LaunchProviderRun: { session_id: state.session_id, agent_id: state.agent_id, adapter_key: 'codex', provider: 'codex', account_profile: state.profile, model: 'gpt-6-luna', variant: 'low', native_tui: false } }).catch(error => log('relaunch', { error: String(error).slice(0, 200) }))
    await rpc({ UpdateAgentTrigger: { session_id: state.session_id, trigger_id: state.http_id, busy_policy: 'queue' } })
    log('http-online-after-relaunch', await waitFor('agent online', async () => { const r = await fetch(state.http_url, { method: 'HEAD' }); return r.status < 500 ? r.status : false }, 120_000))
    const restartPost = await post(state.http_url, { marker: 'AFTER_RESTART_OK' })
    log('http-post-after-restart', restartPost)
    accepted('post-after-restart-accepted', restartPost, ['started', 'queued'])
    log('http-answered-after-restart', await waitFor('answer after restart', () => answered('AFTER_RESTART_OK'), 240_000))
    await rpc({ UpdateAgentTrigger: { session_id: state.session_id, trigger_id: state.http_id, enabled: false } })
    const disabledPost = await post(state.http_url, { marker: 'DISABLED' })
    log('http-disabled-post', disabledPost)
    expect('disabled-post-403', disabledPost.status === 403, { status: disabledPost.status })
    await rpc({ DeleteAgentTrigger: { session_id: state.session_id, trigger_id: state.http_id } })
    await delay(3000)
    const afterDelete = await fetch(state.http_url, { method: 'POST', body: '{}' }).then(r => ({ status: r.status })).catch(error => ({ refused: String(error.cause?.code ?? error) }))
    log('http-after-delete', afterDelete)
    expect('deleted-connection-refused', afterDelete.refused === 'ECONNREFUSED', afterDelete)
    await rpc({ ArchiveAgent: { session_id: state.session_id, agent_id: state.agent_id, archived: true } })
    const left = (await triggers()).length
    log('archived', { triggers_left: left })
    expect('archive-removes-every-trigger', left === 0, { triggers_left: left })
  }
  state[`${phase}_answers`] = await answers()
  state[`${phase}_ok`] = true; save()
} catch (error) {
  log('error', { error: String(error?.stack ?? error).slice(0, 600) })
  process.exitCode = 1
} finally {
  await client.close().catch(() => {})
}
