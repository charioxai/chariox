// MP-08/MP-10/MP-11: frozen prompt/budgets and official substitute judge, fresh Room.
import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { randomUUID, createHash } from 'node:crypto'
import { captureFinalEvidence } from './webvoyager-final-capture.mjs'
import { Round2Room } from './round2/room.mjs'
import { unwrap, getTurn, loadTurnHistory, assembleTurnEntries } from './round2/kernel.mjs'
import { finalAnswer, settlementRecord } from './round2/export.mjs'
import { providerFailureFlags } from './round2/provider-availability.mjs'
import { observeKernelRpcErrors, rpcErrorRecord } from './round2/rpc-errors.mjs'
import { submitOnce } from './webvoyager-admission.mjs'
import { judgeWebVoyager } from './webvoyager-linked-judge.mjs'
import { roomProviderToolName } from '../lib/room-provider-tool-record.mjs'
import { sanitizeDrillMetadata } from '../lib/drill-secrets.mjs'
const exec = promisify(execFile), pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const observations = new Set(['browser_status', 'browser_find', 'browser_text', 'browser_wait_for_text', 'browser_wait_for_selector', 'browser_wait_for_idle', 'browser_events', 'browser_downloads', 'browser_history'])

export function webVoyagerPrompt(task, variant = 'frozen') {
  assert(['frozen', 'recovery-v1', 'recovery-vision-v1'].includes(variant), 'MP-10 unknown prompt variant')
  const frozen = `MP-08 / MP-10 WebVoyager diagnostic smoke. Solve this task in the Room browser.\nTask: ${task.ques}\nStart URL: ${task.web}\nUse ONLY Chariox first-party slice_open_url and slice_browser_* tools. Open the start URL first. Observe before acting and use observed opaque field IDs. No shell, files, scripts, direct HTTP, provider-native browser, other MCPs, Computer tools, benchmark code or expected answers. Read-only browsing only: no logins, sign-ups, purchases, cart changes, bookings, posts, messages, uploads, account changes or other submissions that change state. Search/filter forms that only retrieve public information are allowed. If the task needs a forbidden action, stop and state SKIPPED_STATE_CHANGE. If blocked by login, CAPTCHA, robots, access denial, expired dates or missing data, stop and state the blocker. Do not bypass restrictions or change the task. Stay within 15 mutating Browser actions, 80 Browser tool calls and 600 seconds. Return a concise factual final answer with source URLs; state failures honestly.`
  if (variant === 'frozen') return frozen
  const recovery = frozen.replace('If blocked by login, CAPTCHA, robots, access denial, expired dates or missing data, stop and state the blocker.',
    'You may dismiss sign-in invitations and informational overlays without signing in, and reject nonessential cookies using observed controls such as Reject all, I do not agree, or necessary-only settings. These privacy-preserving navigation choices are permitted; do not stop solely because they save a local browser preference. Do not accept optional tracking. Do not change delivery addresses or invent a location. If blocked by mandatory login, CAPTCHA, robots, access denial or expired dates, stop and state the blocker. Before declaring missing data, wait for the requested result and read the relevant section. For image-only results, look for an observed Plain Text or accessible alternative; never infer values from an unreadable image.')
    + '\nNavigation recovery: if an element is obscured or disabled, inspect the current page for a dismissible overlay before retrying. Refresh observed field IDs after navigation or a stale-element error. Use the visible control, not a hidden duplicate. Allow one recovery attempt after a wait for idle; avoid repeating an unchanged failing action. Read the required content with targeted browser_text queries and pagination within the tool limits. Verify each requested filter, date, unit, count and ranking on the resulting page before answering. For latest/recent items verify dates and compare candidates. Give only verified facts and identify any unmet requirement; never claim full success from a partial result.'
  if (variant === 'recovery-v1') return recovery
  return recovery.replace('Use ONLY Chariox first-party slice_open_url and slice_browser_* tools.',
    'Round 4 vision-allowed. Use ONLY Chariox first-party slice_open_url, slice_browser_* and protected slice_screenshot tools. Use slice_screenshot with return_image_base64=true through the ordinary Room/kernel path to see the current page; do not supply a path or read artifact files.')
    .replace('other MCPs, Computer tools,', 'other MCPs, Computer input tools,')
    .replace('For image-only results, look for an observed Plain Text or accessible alternative; never infer values from an unreadable image.',
      'For image-only results, inspect a fresh protected screenshot and any observed Plain Text or accessible alternative. Read values, units and labels carefully; never infer values from an unreadable image.')
}

export function webVoyagerToolAllowed(tool, variant = 'frozen') {
  return /^(slice_open_url|slice_browser_[a-z_]+)$/.test(tool)
    || variant === 'recovery-vision-v1' && tool === 'slice_screenshot'
}

export async function runWebVoyagerTask({ task, runtime, options }) {
  const id = `${options.runtime.laneName ?? 'r2next'}-wv-${randomUUID().slice(0, 8)}`
  const directory = `${options.runtime.evidence}/attempts/${task.id}`
  await mkdir(directory, { recursive: false, mode: 0o700 })
  const promptVariant = options.promptVariant ?? 'frozen'
  const prompt = webVoyagerPrompt(task, promptVariant)
  const row = { ...(promptVariant === 'recovery-vision-v1' ? { observationPolicy: 'vision-allowed', round: 4 } : {}), promptVariant, promptSha256: createHash('sha256').update(prompt).digest('hex'), mpItems: ['MP-08', 'MP-10', 'MP-11'], benchmark: 'WebVoyager', scope: options.scope ?? 'round3-full',
    taskId: task.id, runId: id, taskRevision: '5a7896738c10bfb8b9edccce6bb0e0411f8ae569',
    model: 'gpt-6.1-sol', effort: 'high', judgeModel: 'gpt-6.1-sol', judgeEffort: 'low',
    source: runtime.source, dateStatus: task.dateStatus, maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000,
    sandboxCompatibility: false, agentPlacement: 'home kernel', startedAt: new Date().toISOString(),
    harnessValid: false, judgeValid: false, screenshots: [], screenshotMetadata: [], captureErrors: [] }
  const checkpoint = () => writeFile(`${directory}/row.json`, JSON.stringify(sanitizeDrillMetadata(row), null, 2) + '\n', { mode: 0o600 })
  const [{ LocalIpcClient }, requests, helpers] = await Promise.all(['ipc', 'ipc-requests', 'session-history-fragments'].map(name => import(pathToFileURL(`${options.runtime.clientRoot}/dist/${name}.js`))))
  const client = new LocalIpcClient(runtime.kernelUrl, { controlResponseStallMs: 240000 })
  const api = { client, requests, helpers }
  let seam = 'room_create', container, volumes = [], agentId, turn, identity
  observeKernelRpcErrors(client, async error => { (row.rpcErrors ??= []).push({ seam, ...error }); await checkpoint() })
  const docker = args => exec('docker', args, { timeout: 60000, maxBuffer: 1024 * 1024 })
  const room = new Round2Room(api, { workspace: runtime.workspace, runId: id,
    checkpoint: async owned => { row.owned = structuredClone(owned); await checkpoint() } })
  async function capture({ admission = false } = {}) {
    const name = admission ? 'admission.png' : `screenshot${row.screenshots.length + 1}.png`
    const remoteName = admission ? 'screenshot0.png' : name
    try {
      const metadata = JSON.parse((await docker(['exec', container, 'node', '/tmp/benchwv-screenshot.mjs', `/tmp/benchwv-${remoteName}`])).stdout)
      await docker(['cp', `${container}:/tmp/benchwv-${remoteName}`, `${directory}/${name}`])
      if (admission) row.admissionScreenshot = { name, metadata }
      else { row.screenshots.push(name); row.screenshotMetadata.push(metadata) }
    } catch (error) {
      // The capture helper exports only allowlisted method/code diagnostics.
      const diagnostics = (error.stderr ?? '').split('\n').map(line => { try { return JSON.parse(line) } catch { return null } })
        .filter(value => value?.mpItems?.includes('MP-10') && typeof value.errorClass === 'string')
        .map(value => ({ errorClass: value.errorClass, method: value.method, code: value.code }))
      row.captureErrors.push({ at: new Date().toISOString(), seam, code: error.code ?? null, diagnostics })
      await checkpoint(); throw Error('MP-10 screenshot capture failed')
    }
  }
  const started = Date.now()
  try {
    await room.setup({ guard: runtime.guard,
      viewport: { css_width: 1024, css_height: 768, device_scale_factor: 1, desktop_pixel_width: 1024, desktop_pixel_height: 768 },
      verifySlice: async slice => {
        seam = 'slice_verify'; container = `chariox-slice-${slice.name}`
        const object = JSON.parse((await docker(['inspect', container])).stdout)[0]
        assert.equal(object.Image, options.runtime.image)
        assert.equal(object.Config.Labels['io.chariox.slice.id'], slice.sliceId)
        assert.equal(object.HostConfig.Memory, 2048 * 1024 ** 2); assert.equal(object.HostConfig.NanoCpus, 1e9)
        volumes = object.Mounts.filter(mount => mount.Type === 'volume').map(mount => mount.Name)
        row.slice = { ...slice, container, volumes, ownerKernelId: object.Config.Labels['io.chariox.slice.owner-kernel-id'] }
        row.browserVersion = (await docker(['exec', container, 'chromium', '--version'])).stdout.trim()
        await docker(['cp', path.join(import.meta.dirname, 'webvoyager-screenshot.mjs'), `${container}:/tmp/benchwv-screenshot.mjs`])
      } })
    seam = 'capture_preflight'; await capture({ admission: true })
    seam = 'agent_spawn'
    agentId = await room.spawn({ provider: 'codex', model: 'gpt-6.1-sol', effort: 'high', accountProfile: runtime.profileId })
    row.sessionId = room.owned.sessionId; row.agentId = agentId
    seam = 'prompt_submit'
    const submitted = unwrap(await submitOnce({ directory: `${options.runtime.evidence}/admissions`, scope: options.scope ?? 'round3-full', taskId: task.id, runId: id, observationPolicy: row.observationPolicy,
      submit: () => client.send(requests.submitPromptRequest(room.owned.sessionId, room.owned.attachmentId, agentId, prompt, [])) }), 'PromptSubmitted')
    row.promptId = (submitted.outcome.Started ?? submitted.outcome.Queued).prompt.id
    identity = { sessionId: room.owned.sessionId, agentId, promptId: row.promptId }
    row.providerStartedAt = new Date().toISOString(); await checkpoint()
    const providerStart = Date.now(); let actions = [], lastCapture = 0, cancelled = false
    seam = 'provider_settlement'
    while (Date.now() - providerStart < 600000) {
      await runtime.guard()
      await client.send({ PumpTerminalOutput: { session_id: identity.sessionId, attachment_id: room.owned.attachmentId } })
      turn = await getTurn(api, identity)
      actions = unwrap(await client.send(requests.listRoomEnvironmentActionHistoryRequest(identity.sessionId, null, 1000)), 'RoomEnvironmentActionHistoryListed').page.actions.filter(action => action.actor_id === `agent:${agentId}`)
      row.mutatingActions = actions.filter(action => !observations.has(action.kind)).length
      if (row.mutatingActions > 15) { row.budgetExceeded = 'mutating_actions'; break }
      if (Date.now() - lastCapture >= 5000) {
        // A transient passive sample must not abort a still-owned solver run.
        // Retain each original capture error; final evidence remains strict.
        await capture().catch(() => {}); lastCapture = Date.now()
      }
      if (['completed', 'failed', 'cancelled'].includes(turn?.lifecycle)) break
      await checkpoint(); await pause(1000)
    }
    row.providerWallSeconds = (Date.now() - providerStart) / 1000
    if (!['completed', 'failed', 'cancelled'].includes(turn?.lifecycle) || row.budgetExceeded) {
      row.budgetExceeded ??= 'wall_time'; cancelled = true; await room.cancel()
      for (let i = 0; i < 120; i++) {
        turn = await getTurn(api, identity)
        if (['completed', 'failed', 'cancelled'].includes(turn?.lifecycle)) break
        await pause(1000)
      }
    }
    row.turnLifecycle = turn?.lifecycle ?? null
    seam = 'tool_audit'; assert(turn, 'MP-10 missing original turn')
    const originals = await loadTurnHistory(api, { ...identity, turn })
    const entries = assembleTurnEntries(originals, helpers), tools = new Map()
    for (const item of entries.filter(item => item.entry.kind === 'provider_tool')) {
      const record = JSON.parse(item.entry.text); tools.set(record.id ?? `missing-${tools.size}`, record)
    }
    row.toolTrace = [...tools.values()].map(record => ({ tool: roomProviderToolName(record.tool), status: record.status, input: sanitizeDrillMetadata(record.input) }))
    row.providerToolCalls = row.toolTrace.length
    row.forbiddenTools = row.toolTrace.filter(tool => !webVoyagerToolAllowed(tool.tool, promptVariant)).map(tool => tool.tool)
    row.providerError = entries.some(item => item.entry.kind === 'provider_error')
    row.providerUnauthorized = entries.some(item => item.entry.kind === 'provider_error' && /401|unauthorized|refresh_token_reused/i.test(item.entry.text))
    row.providerErrors = settlementRecord({ turn, entries: originals, elapsedMs: Date.now() - providerStart, ...identity, helpers }).providerErrors
    row.providerUsageExhausted = entries.some(item => item.entry.kind === 'provider_error' && providerFailureFlags(item.entry.text).usageExhausted)
    if (row.providerUnauthorized) console.log('MP-08/MP-10 coordinator notice: linked provider unauthorized')
    if (row.providerError || turn.lifecycle !== 'completed') {
      seam = 'provider_settlement'; await checkpoint()
      throw Error('MP-08/MP-10 provider failed; original settlement errors retained')
    }
    row.answer = finalAnswer(turn, originals, helpers)
    await writeFile(`${directory}/answer.txt`, row.answer, { mode: 0o600 })
    row.actions = actions.map(action => ({ id: action.action_id, kind: action.kind, mode: action.mode, state: action.state, actorId: action.actor_id }))
    seam = 'final_capture'; row.finalCapture = await captureFinalEvidence({ capture })
    row.harnessValid = !cancelled && turn.lifecycle === 'completed' && !row.providerError && !!row.answer
      && !!tools.size && !row.forbiddenTools.length && tools.size <= 80 && !!actions.length && actions.every(action => action.mode === 'browser')
    const state = unwrap(await client.send(requests.getSessionStateRequest(identity.sessionId)), 'SessionState')
    const run = state.agent_activity?.[agentId]?.last_completed_turn?.provider_run_id
    row.usageTokensTotal = run ? unwrap(await client.send(requests.getProviderRunRequest(run)), 'ProviderRun').provider_run.usage_tokens_total : null
    seam = 'official_prompt_judge'
    Object.assign(row, await judgeWebVoyager({ task, directory, screenshots: row.screenshots, upstream: options.upstream,
      root: runtime.root, accountHome: options.runtime.accountHome, observationPolicy: row.observationPolicy }))
  } catch (error) {
    row.firstFailingSeam = seam; row.failure = rpcErrorRecord(error)
    if (error.judgeFailure) row.judgeFailure = error.judgeFailure
    row.providerUsageExhausted ||= providerFailureFlags(error.message).usageExhausted
    if (seam !== 'official_prompt_judge') row.harnessValid = false
  } finally {
    if (agentId) {
      try {
        const state = unwrap(await client.send(requests.getSessionStateRequest(room.owned.sessionId)), 'SessionState')
        if (state.session.agents.find(agent => agent.id === agentId)?.is_processing) {
          await room.cancel()
          for (let i = 0; i < 120; i++) {
            const state = unwrap(await client.send(requests.getSessionStateRequest(room.owned.sessionId)), 'SessionState')
            if (!state.session.agents.find(agent => agent.id === agentId)?.is_processing) break
            await pause(1000)
          }
        }
      } catch { row.cancelFailed = true }
    }
    row.cleanup = await room.cleanup()
    row.containerGone = container ? await docker(['inspect', container]).then(() => false, () => true) : true
    row.volumesGone = (await Promise.all(volumes.map(volume => docker(['volume', 'inspect', volume]).then(() => false, () => true)))).every(Boolean)
    row.cleanupValid = row.cleanup.complete && row.containerGone && row.volumesGone && !row.cancelFailed && !row.judgeFailure?.cleanupFailed
    if (!row.cleanupValid) row.harnessValid = false
    row.finishedAt = new Date().toISOString(); row.elapsedMs = Date.now() - started
    await checkpoint(); await client.close()
  }
  return row
}
