// MP-08 / MP-09 / MP-10 / MP-11: a real Codex agent in a plain Room (no workflow) uses the Vault management tool
// and a blocking popup. Cells: A status, B popup (owner types the passphrase), C blocking popup whose waiting call
// is cut by the provider: the card must not stay behind. Run with the base kernel for RED and the fixed one for GREEN.
// Run as root or any user that can read the linked Codex profile; no hosted stack, browser or relay client is needed.
import assert from 'node:assert/strict'
import {mkdir, writeFile} from 'node:fs/promises'
import path from 'node:path'
import {startKernelFixture, waitFor} from '/root/work/agent-vflag-cloud/apps/web/scripts/lib/external-access-kernel-fixture.mjs'

const binaryDir = process.env.VFLAG_BIN, evidence = process.env.VFLAG_EVIDENCE, profileHome = process.env.VFLAG_CODEX_PROFILE
const cells = (process.env.VFLAG_CELLS ?? 'A,B,C').split(',')
assert.ok(binaryDir && evidence && profileHome && path.isAbsolute(evidence), 'Set VFLAG_BIN, VFLAG_EVIDENCE (absolute) and VFLAG_CODEX_PROFILE')
await mkdir(evidence, {recursive: true})
const report = {MP: 'MP-08 / MP-09 / MP-10 / MP-11', binaryDir, cells: {}}
let fixture
const text = bytes => Buffer.from(bytes).toString('utf8')

try {
  fixture = await startKernelFixture(binaryDir, '/usr/local/bin/codex', {kernelEnvironment: {CHARIOX_ROOM_AGENT_TOOLS: '1'}})
  const profile = (await fixture.send({LinkProviderAccountProfile: {provider: 'codex', label: 'vflag tool drill', path: profileHome}})).ProviderAccountProfile.profile
  const worktrees = (await fixture.send({ListWorkspaceWorktrees: {workspace_id: fixture.workspace}})).WorkspaceWorktreesListed.worktrees
  const created = (await fixture.send({CreateSession: {workspace_id: fixture.workspace, worktree_id: worktrees[0].path, alias: 'vflag vault tool', agent_defaults: {provider: 'codex', model: 'gpt-6-luna', effort: 'low', account_profile: profile.profile_id}}})).SessionCreated
  const sessionId = created.session.id, agentId = created.agent.id
  const attachment = (await fixture.send({AttachToSession: {session_id: sessionId, client_id: 'vflag-tool-drill', capability_level: 'FullTerminal'}})).SessionAttached.attachment.id
  const records = []
  // An idle attachment is reaped, so one loop keeps pumping for the whole run.
  let running = true
  const pump = async () => { for (const record of (await fixture.send({PumpTerminalOutput: {session_id: sessionId, attachment_id: attachment}})).TerminalOutput.records) records.push({at: Date.now(), ...record}) }
  const pumping = (async () => { while (running) { await pump().catch(() => {}); await new Promise(resolve => setTimeout(resolve, 1500)) } })()
  const cards = async () => (await fixture.send({ListSessions: {}})).SessionsListed.sessions.find(item => item.id === sessionId).active_interactions ?? []
  const ask = async (prompt, done, seconds = 240) => {
    const from = records.length
    await fixture.send({SubmitPrompt: {session_id: sessionId, attachment_id: attachment, target_agent_id: agentId, prompt, attachments: []}})
    const end = Date.now() + seconds * 1000
    while (Date.now() < end) {
      const fresh = records.slice(from)
      const output = fresh.filter(item => item.kind === 'provider_output').map(item => text(item.bytes)).join('')
      const tools = fresh.filter(item => item.kind === 'provider_tool').map(item => text(item.bytes))
      if (done(output, tools)) return {output, tools}
      await new Promise(resolve => setTimeout(resolve, 1500))
    }
    throw Error('agent did not finish in time')
  }
  const toolError = tools => tools.find(tool => /manage_credential_vault/.test(tool) && /"status":"error"/.test(tool))
  report.agent = {model: 'gpt-6-luna', effort: 'low'}

  if (cells.includes('A')) {
    const {output, tools} = await ask('Call the tool chariox.manage_credential_vault with action status, once. Then reply with the two words VAULTSTATUS and CHECKED joined by an underscore, then true or false for the unlocked field, or the word ERROR if the tool failed.', out => /VAULTSTATUS_CHECKED\s*(true|false|ERROR)/.test(out))
    const failed = toolError(tools)
    report.cells.A = {pass: !failed && /VAULTSTATUS_CHECKED\s*(true|false)/.test(output), reply: output.trim().slice(0, 80), toolError: failed ? /no active workflow turn/.test(failed) ? 'no active workflow turn for authenticated provider run' : 'other' : null}
  }
  if (cells.includes('B')) {
    const answered = (async () => {
      const card = await waitFor(async () => (await cards()).find(item => item.id.startsWith('vault-unlock-')), 'vault unlock card', 120000).catch(() => null)
      if (!card) return null
      const opened = Date.now()
      await fixture.send({RespondToInteraction: {session_id: sessionId, interaction_id: card.id, choice_id: 'passphrase', custom_reply: fixture.passkey}})
      return {timeoutSec: card.timeout_sec, secondsToAnswer: Math.round((Date.now() - opened) / 1000)}
    })()
    const {output, tools} = await ask('Call the tool chariox.manage_credential_vault with action popup, once, and wait for it to return. Then reply with the two words POPUP and RETURNED joined by an underscore, or the word ERROR if the tool failed.', out => /POPUP_RETURNED|ERROR/.test(out), 200)
    const failed = toolError(tools)
    report.cells.B = {pass: !failed && /POPUP_RETURNED/.test(output), card: await answered, toolError: failed ? 'failed' : null}
  }
  if (cells.includes('C')) {
    const asked = Date.now()
    let seen = null, goneAfter = null
    const watch = (async () => {
      for (;;) {
        const card = (await cards()).find(item => item.title === 'dead card test')
        if (card && !seen) seen = Date.now()
        if (seen && !card) { goneAfter = Math.round((Date.now() - seen) / 1000); return }
        if (Date.now() - asked > 600000) return
        await new Promise(resolve => setTimeout(resolve, 2000))
      }
    })()
    await fixture.send({SubmitPrompt: {session_id: sessionId, attachment_id: attachment, target_agent_id: agentId, prompt: 'Call chariox.request_popup once with title "dead card test", message "wait", one choice {"id":"done","label":"Done","reply":"Done"} and the default blocking wait. Do nothing else and do not call it again.', attachments: []}})
    await watch
    const popupCalls = records.filter(item => item.kind === 'provider_tool' && /request_popup/.test(text(item.bytes))).map(item => ({secondsAfterAsk: Math.round((item.at - asked) / 1000), status: /"status":"(\w+)"/.exec(text(item.bytes))?.[1], error: /timed out awaiting tools\/call after (\d+)s/.exec(text(item.bytes))?.[0] ?? null}))
    const stillOpen = (await cards()).some(item => item.title === 'dead card test')
    report.cells.C = {popupCalls, pass: goneAfter !== null && !stillOpen, cardSeen: Boolean(seen), cardOpenedSeconds: seen ? Math.round(((goneAfter === null ? Date.now() : seen + goneAfter * 1000) - seen) / 1000) : null, stillOpenAfter10Minutes: stillOpen}
  }
  running = false
  await pumping
  report.status = Object.values(report.cells).every(cell => cell.pass) ? 'PASS' : 'FAIL'
} catch (error) {
  report.status = 'ERROR'; report.error = String(error.message).slice(0, 400)
} finally {
  await fixture?.close()
  await writeFile(path.join(evidence, `vault-tool-${process.env.VFLAG_LABEL ?? 'run'}.json`), JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(report))
}
