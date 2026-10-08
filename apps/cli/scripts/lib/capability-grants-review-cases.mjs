// MP-08/MP-10/MP-11 #922 R1–R3. Real TUI prompts/decisions and official
// Codex, with public Chariox RPCs for the external editor and run replacement.
// These local seam drills do not establish hosted/public-site acceptance.
import { randomBytes } from 'node:crypto'

export async function runCapabilityReviewCase(kind, ctx) {
  const { client, requests, automation, sessionId, profileId, options, report,
    step, passed, requireValue, until, press, visibleScreen, capture, requestBrowser } = ctx
  report.reviewCase = kind
  const grants = async () => (await client.send(requests.kernelBrowserRequest({ op: 'list_grants' }))).KernelBrowser.result.grants
  const snapshot = () => automation.send('snapshot')
  const state = async () => (await client.send(requests.getSessionStateRequest(sessionId))).SessionState.session
  const active = (session, agent) => session.prompt_states?.[agent]?.active_prompt
    ?? session.active_prompt // Kept for older session projections.
  const decide = async (choice) => {
    await press(choice === 'allow' ? '\x1b[B' : '\x1b[A')
    await until(() => visibleScreen().includes(`Selected: ${choice === 'allow' ? 'Allow' : 'Deny'}`), 5_000)
    await press('\r')
  }
  if (kind === 'r1') {
    step('r1-owner-initializes-disposable-vault-through-tui')
    // Fresh lane-owned kernel and HOME: normal product initialization, never
    // an operator credential or identity copied from another kernel.
    const passkey = randomBytes(24).toString('hex')
    ctx.registerPrivateInput(passkey)
    const manage = automation.send('submit_prompt', { prompt: '/credential vault manage' })
    let unlock
    await until(async () => { unlock = (await snapshot()).interactions.find(i => i.title === 'Unlock Chariox Vault'); return Boolean(unlock) }, 30_000)
    requireValue(unlock.customChoice?.input_kind === 'secret')
    await automation.send('interaction_custom_reply', { interactionId: unlock.id, reply: passkey })
    await automation.send('interaction_move', { delta: 1 })
    await automation.send('interaction_submit')
    let duration
    await until(async () => {
      duration = (await snapshot()).interactions.find(i => i.title === 'Choose Vault Unlock Duration')
      return Boolean(duration) || (await client.send(requests.getCredentialVaultStatusRequest())).CredentialVaultStatus.status.unlocked
    }, 30_000)
    if (duration) {
      const delta = duration.choices.findIndex(c => c.id === 'unlock_kernel') - (duration.selectedChoiceIndex ?? 0)
      if (delta) await automation.send('interaction_move', { delta })
      await automation.send('interaction_submit')
    }
    await manage
    const vault = await client.send(requests.getCredentialVaultStatusRequest())
    requireValue(vault.CredentialVaultStatus.status.unlocked === true)
    passed('r1-owner-initializes-disposable-vault-through-tui')
    const external = new ctx.LocalIpcClient(`ws+unix://${options.home}/daemon.sock`, { controlRequestRetryDeadlineMs: 0 })
    try {
      step('r1-owner-grants-real-external-process-through-tui')
      const access = external.send({ RequestKernelAccess: { session_id: sessionId, holder_pid: process.pid, lifetime_minutes: 10 } })
      access.catch(() => {})
      await until(async () => (await snapshot()).interactions.some(i => i.title === 'Grant external agent access'), 30_000)
      // The passkey popup is the normal F8 surface; secret input is masked.
      await press('\x1b[19~')
      await until(() => visibleScreen().includes('Chariox passkey'), 30_000)
      await capture('r1-passkey-popup-tui.ansi')
      await press(passkey)
      await press('\r')
      await capture('r1-passkey-submitted-tui.ansi')
      await Promise.race([access, Bun.sleep(15_000).then(() => { throw new Error('MP-11 R1 external access approval did not settle') })])
      passed('r1-owner-grants-real-external-process-through-tui')
      const attached = await external.send({ AttachToSession: { session_id: sessionId, client_id: `am5-review-editor-${process.pid}`, capability_level: 'AutomationOnly' } })
      const attachment = attached.SessionAttached.attachment
      requireValue(attachment.capability_level === 'AutomationOnly')
      const approval = await requestBrowser('r1-original-owner-request-tui.ansi')
      const agent = (await snapshot()).session.focusedAgentId
      step('r1-owner-queues-prompt-through-tui')
      await automation.send('submit_prompt', { prompt: 'For my queued request, use the Chariox browser to open https://www.wikipedia.org/. First call load_kernel_browser once. If refused, stop and quote its exact error.' })
      let queued
      await until(async () => { queued = (await snapshot()).queuedPromptStrips[agent]?.items.at(-1); return Boolean(queued) }, 10_000)
      await capture('r1-human-queued-tui.ansi')
      step('r1-external-edits-human-queued-prompt')
      const text = 'AM5_EXTERNAL_EDIT: Use Chariox load_kernel_browser exactly once with no arguments to open https://www.wikipedia.org/. If refused, stop and quote the exact error. Do not use any other tool.'
      await external.send(requests.updateQueuedPromptRequest(sessionId, attachment.id, agent, queued.promptId, text))
      await until(async () => (await snapshot()).queuedPromptStrips[agent]?.items.some(i => i.prompt === text), 10_000)
      await capture('r1-external-edited-queue-tui.ansi')
      passed('r1-external-edits-human-queued-prompt')
      step('r1-owner-denies-original-request-through-tui')
      await decide('deny')
      await until(async () => !(await snapshot()).interactions.some(i => i.id === approval.id), 15_000)
      step('r1-promoted-external-edit-does-not-open-owner-acquisition')
      await until(async () => {
        const view = await snapshot()
        if (view.interactions.some(i => i.title === 'Chariox resource access')) {
          report.firstFailingSeam = 'R1: promoted external edit retains owner acquisition provenance'
          await capture('r1-stale-owner-acquisition-tui.ansi')
          throw new Error('MP-11 R1 external edit opened an owner acquisition popup')
        }
        const entries = view.agentPanes[agent] ?? view.transcript.entries
        const editedPrompt = entries.findLastIndex(entry => entry.role === 'user' && entry.text.includes('AM5_EXTERNAL_EDIT'))
        return editedPrompt >= 0 && !view.session.agents.find(a => a.id === agent).isProcessing
          && !view.queuedPromptStrips[agent]?.items.length
          && entries.slice(editedPrompt + 1).some(entry => entry.text.includes('user_domain_not_requested'))
      }, 180_000)
      requireValue((await grants()).length === 0)
      await capture('r1-promoted-edit-refused-tui.ansi')
      passed('r1-promoted-external-edit-does-not-open-owner-acquisition', { noGrant: true, refusalCode: 'user_domain_not_requested' })
    } finally { await external.close() }
  } else if (kind === 'r2') {
    step('r2-official-codex-requests-browser-through-tui')
    const approval = await requestBrowser('r2-before-replacement-tui.ansi')
    const agent = (await snapshot()).session.focusedAgentId
    const before = await state()
    const prompt = active(before, agent)
    requireValue(prompt?.id)
    step('r2-public-provider-launch-replaces-caller-with-same-owner-prompt')
    // Fault injection through the public launch contract. The actual prompt,
    // provider MCP request and owner popup came from the real TUI/Codex path.
    report.replacementBefore = { promptId: prompt.id, runId: before.active_provider_run_id }
    let replacement
    try { replacement = await client.send(requests.launchProviderRunRequest(sessionId, 'codex', profileId, 'gpt-6.1-sol', 'low', agent)) }
    catch (error) {
      report.replacementErrorCode = error.code ?? null
      report.replacementBlockedReason = String(error.message).match(/provider run `[A-Za-z0-9_-]+` cannot perform `spawn current provider launch` while starting/)?.[0] ?? 'public provider launch failed'
      report.livePrerequisiteBlocked = true
      throw error
    }
    report.replacementResponseKinds = Object.keys(replacement)
    const replacedRun = replacement.ProviderRunLaunched?.provider_run ?? replacement.ProviderRunLaunchAccepted?.provider_run
    requireValue(replacedRun?.id)
    const after = await state()
    report.replacementAfter = { promptId: active(after, agent)?.id, runId: after.active_provider_run_id }
    requireValue(active(after, agent)?.id === prompt.id)
    report.samePromptRetainedAtReplacement = true
    await capture('r2-replaced-run-tui.ansi')
    step('r2-stale-browser-popup-withdraws-without-owner-reply')
    try {
      await until(async () => !(await snapshot()).interactions.some(i => i.id === approval.id), 5_000)
    } catch {
      report.firstFailingSeam = 'R2: same-prompt caller replacement leaves browser approval pending'
      throw new Error('MP-11 R2 stale browser popup was not withdrawn')
    }
    requireValue((await grants()).length === 0)
    await capture('r2-stale-popup-withdrawn-tui.ansi')
    passed('r2-stale-browser-popup-withdraws-without-owner-reply', { samePrompt: prompt.id, noGrant: true })
  } else if (kind === 'r3') {
    step('r3-official-codex-requests-new-tabs-grant-through-tui')
    const prompt = 'First call Chariox load_kernel_browser exactly once with no arguments. If it succeeds, call Chariox kernel_browser with command {"op":"stop"} exactly once. Stop after that and quote the exact error including its error code. Do not use other tools.'
    const agent = (await snapshot()).session.focusedAgentId
    await automation.send('submit_prompt', { prompt })
    await until(async () => (await snapshot()).interactions.some(i => i.title === 'Chariox resource access'), 180_000)
    await press('\x1b[19~')
    await until(() => visibleScreen().includes('Chariox resource access'), 10_000)
    await capture('r3-new-tabs-approval-tui.ansi')
    await decide('allow')
    await until(async () => (await grants()).length === 1, 15_000)
    step('r3-provider-stop-keeps-typed-not-focused-refusal')
    try {
      await until(async () => {
        const view = await snapshot()
        const entries = view.agentPanes[agent] ?? view.transcript.entries
        const currentPrompt = entries.findLastIndex(entry => entry.role === 'user' && entry.text === prompt)
        if (currentPrompt < 0 || view.session.agents.find(a => a.id === agent).isProcessing) return false
        const response = entries.slice(currentPrompt + 1).map(entry => entry.text).join('\n')
        if (response.includes('user_domain_not_focused_agent')) return true
        if (response.includes('browser stop requires live owner focus')) {
          report.providerStopObserved = true
          throw new Error('MP-11 R3 Stop returned an untyped refusal')
        }
        return false
      }, 180_000)
    }
    catch {
      report.firstFailingSeam = 'R3: provider Stop did not retain user_domain_not_focused_agent'
      throw new Error('MP-11 R3 Stop refusal is not typed')
    }
    requireValue((await grants())[0].focused === false)
    await capture('r3-stop-typed-refusal-tui.ansi')
    passed('r3-provider-stop-keeps-typed-not-focused-refusal', { refusalCode: 'user_domain_not_focused_agent' })
  } else { throw new Error('review-case must be r1, r2 or r3') }
  report.status = 'passed'
  report.scope = `real local TUI/kernel/official Codex #922 ${kind} seam; hosted acceptance remains separate`
  report.reviewCase = kind
  report.blockers = ['MP-10: isolated #922 hosted stack, real production App installation/account, ordinary-user browser/public-site DPR1/2 shaped-network and multi-hour acceptance remain required.']
}
