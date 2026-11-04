// MP-08 / MP-09 / MP-10 / MP-11: a real Codex agent captures a label (the 3-letter word "new" on a real page) with
// chariox.vault.capture_browser by mistake, then captures a value on the same page, whose URL contains that word,
// then removes the label capture with chariox.vault.remove_capture and reads the page again.
// Base kernel (RED): the label is protected everywhere, the page URL is redacted for the kernel, the second capture fails
// and there is no remove tool. Fixed kernel (GREEN): the short word is masked as a whole word only, the second capture
// works, the agent removes its own capture and the word reads normally again.
// Env: ROOM_KERNEL_ROOT (kernel dir with ports.json and home/), R6_WORKSPACE (git repo path the kernel may use),
// R6_PROFILE (Codex profile dir the kernel user can read), R6_PASSKEY_FILE (new Vault passphrase), R6_EVIDENCE (absolute).
import assert from 'node:assert/strict'
import {mkdir, readFile, writeFile} from 'node:fs/promises'
import path from 'node:path'
import {connect} from './room-driver/rpc.mjs'

const {R6_WORKSPACE: workspace, R6_PROFILE: profileHome, R6_PASSKEY_FILE: passkeyFile, R6_EVIDENCE: evidence, R6_LABEL: label = 'run'} = process.env
assert.ok(workspace && profileHome && passkeyFile && path.isAbsolute(evidence ?? ''), 'Set R6_WORKSPACE, R6_PROFILE, R6_PASSKEY_FILE and R6_EVIDENCE (absolute)')
await mkdir(evidence, {recursive: true})
const passkey = (await readFile(passkeyFile, 'utf8')).trim()
const text = bytes => Buffer.from(bytes).toString('utf8')
const report = {MP: 'MP-08 / MP-09 / MP-10 / MP-11', label}
const peer = await connect()
try {
  // A second run on the same kernel finds the directory already linked; the error names that profile.
  const profile = await peer.send({LinkProviderAccountProfile: {provider: 'codex', label: 'vflag r6 label drill', path: profileHome}})
    .then(reply => reply.ProviderAccountProfile.profile, error => {
      const linked = /\(([a-z0-9-]+)\); use that profile/.exec(error.message)
      if (!linked) throw error
      return {profile_id: linked[1]}
    })
  const worktrees = (await peer.send({ListWorkspaceWorktrees: {workspace_id: workspace}})).WorkspaceWorktreesListed.worktrees
  const created = (await peer.send({CreateSession: {workspace_id: workspace, worktree_id: worktrees[0].path, alias: `vflag label drill ${label}`, agent_defaults: {provider: 'codex', model: 'gpt-6-luna', effort: 'low', account_profile: profile.profile_id}}})).SessionCreated
  const sessionId = created.session.id, agentId = created.agent.id
  const attachment = (await peer.send({AttachToSession: {session_id: sessionId, client_id: 'vflag-label-drill', capability_level: 'FullTerminal'}})).SessionAttached.attachment.id
  const records = []
  let running = true
  const pumping = (async () => {
    while (running) {
      try { for (const record of (await peer.send({PumpTerminalOutput: {session_id: sessionId, attachment_id: attachment}})).TerminalOutput.records) records.push({at: Date.now(), ...record}) } catch {}
      // The Vault is created by the first unlock card; the owner's passphrase answers it.
      try {
        const cards = (await peer.send({ListSessions: {}})).SessionsListed.sessions.find(item => item.id === sessionId).active_interactions ?? []
        for (const card of cards.filter(item => item.id.startsWith('vault-unlock-'))) {
          await peer.send({RespondToInteraction: {session_id: sessionId, interaction_id: card.id, choice_id: 'passphrase', custom_reply: passkey}})
        }
      } catch {}
      await new Promise(resolve => setTimeout(resolve, 1500))
    }
  })()
  const ask = async (prompt, done, seconds) => {
    const from = records.length
    await peer.send({SubmitPrompt: {session_id: sessionId, attachment_id: attachment, target_agent_id: agentId, prompt, attachments: []}})
    for (const end = Date.now() + seconds * 1000; Date.now() < end; await new Promise(resolve => setTimeout(resolve, 2000))) {
      const fresh = records.slice(from)
      const output = fresh.filter(item => item.kind === 'provider_output').map(item => text(item.bytes)).join('')
      if (done(output)) return {output, tools: fresh.filter(item => item.kind === 'provider_tool').map(item => text(item.bytes))}
    }
    throw Error('agent did not finish in time')
  }
  report.agent = {model: 'gpt-6-luna', effort: 'low'}
  const {output, tools} = await ask(
    'MP-08 / MP-09 / MP-10 / MP-11 drill. Load the kernel browser tools if you have not (chariox.load_kernel_browser), open https://news.ycombinator.com/newest with chariox.kernel_browser and take a snapshot. ' +
    'Take a fresh snapshot immediately before each capture and copy its document_id, generation and node_ref exactly. If a capture fails with "snapshot changed target or document identity", take a fresh snapshot and retry it once with a new request_id; never retry any other failure. ' +
    'STEP 1: find the top navigation link whose text is exactly "new" and call chariox.vault.capture_browser on that node (read "node", request_id "label1", label "new link", use_binding {"target":{"kind":"command_environment","name":"DRILL_LABEL","origin":"https://news.ycombinator.com"}}). ' +
    'STEP 2: without navigating away, take a fresh snapshot and call chariox.vault.capture_browser on the title link of the first story in the list (read "node", request_id "value1", label "first story title", same use_binding but name DRILL_VALUE). Report the host field of its result. ' +
    'STEP 3: if step 1 stored an entry, call chariox.vault.remove_capture with its credential_id and report the removed entry (label, length); if it has no such tool say NOTOOL. ' +
    'STEP 4: take a snapshot once more and say whether the top navigation link text reads "new" (VISIBLE) or "[redacted]" (MASKED). ' +
    'Reply on five lines: STEP1 OK or ERROR with the first 100 characters of an error; STEP2 OK or ERROR with the host or the error; STEP3 OK with the label and length, or ERROR/NOTOOL; STEP4 VISIBLE or MASKED; then LABELDRILL_DONE. Never print any captured value.',
    out => /LABELDRILL_DONE/.test(out), 700)
  const step = n => new RegExp(`STEP${n}\\s*[:\\-]?\\s*(OK|ERROR|NOTOOL|VISIBLE|MASKED)([^\\n]*)`).exec(output)
  for (const n of [1, 2, 3, 4]) report[`step${n}`] = step(n)?.slice(1).join(' ').trim()
  // The tool results never carry a Vault value; the reply text is kept to show the refusal or failure class.
  report.captureCalls = tools.filter(tool => /capture_browser/.test(tool)).map(tool => ({
    failed: /"status":"error"|"error":/.test(tool.slice(0, 600)),
    class: /browser_secret_target_changed|document URL is unavailable or invalid|browser_capture_[a-z_]+/.exec(tool)?.[0] ?? null,
    reply: tool.replace(/"(input|arguments)":\{.*?\},/s, '').slice(0, 420),
  }))
  report.removeCalls = tools.filter(tool => /remove_capture/.test(tool)).map(tool => tool.replace(/"(input|arguments)":\{.*?\},/s, '').slice(0, 420))
  running = false; await pumping
  await writeFile(path.join(evidence, `capture-label-${label}.tools.txt`), records.filter(item => item.kind === 'provider_tool').map(item => text(item.bytes).slice(0, 1500)).join('\n'))
  report.status = /^OK/.test(report.step2 ?? '') && /^OK/.test(report.step3 ?? '') && /^VISIBLE/.test(report.step4 ?? '') ? 'GREEN' : 'RED'
} catch (error) {
  report.status = 'ERROR'; report.error = String(error.message).slice(0, 400)
} finally {
  peer.close()
  await writeFile(path.join(evidence, `capture-label-${label}.json`), JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(report))
}
