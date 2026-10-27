#!/usr/bin/env node
// Room terminal attach on the real kernel binary and the real CLI, in a real PTY (a private tmux server is the
// outer terminal): `chariox terminal` and the TUI's `/room terminal` hand the screen to a Room terminal running
// python and vim, Ctrl-] returns, and a re-attach shows the same screen. Needs CHARIOX_KERNEL_BIN (a built
// chariox-kernel) and `pnpm build` output in apps/cli/dist.
import { execFileSync, spawn } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const kernelBin = process.env.CHARIOX_KERNEL_BIN
if (!kernelBin) throw new Error('set CHARIOX_KERNEL_BIN to a built chariox-kernel')
const evidence = process.env.DRILL_EVIDENCE_DIR
const root = mkdtempSync(path.join(process.env.DRILL_ROOT ?? '/var/tmp', 'terminal-attach-drill-'))
const { LocalIpcClient } = await import(path.join(repoRoot, 'packages/kernel-client/dist/ipc.js'))
const requests = await import(path.join(repoRoot, 'packages/kernel-client/dist/ipc-requests.js'))

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const results = []
const check = (ok, what) => { results.push({ ok, what }); console.log(`${ok ? 'PASS' : 'FAIL'} ${what}`) }

const tmuxSocket = `terminal-attach-${process.pid}`
const tmux = (...args) => execFileSync('tmux', ['-L', tmuxSocket, ...args], { encoding: 'utf8' })
const screen = (pane) => tmux('capture-pane', '-p', '-t', pane)
const type = (pane, text) => tmux('send-keys', '-t', pane, '-l', text)
const key = (pane, ...keys) => tmux('send-keys', '-t', pane, ...keys)
const line = (pane, text) => { type(pane, text); key(pane, 'Enter') }
const keep = (label, pane) => {
  const text = screen(pane)
  if (evidence) writeFileSync(path.join(evidence, `${label}.screen.txt`), text)
  return text
}
async function waitFor(pane, predicate, what, ms = 20_000) {
  const deadline = Date.now() + ms
  let last = ''
  while (Date.now() < deadline) {
    last = screen(pane)
    if (predicate(last)) return last
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${what}\n--- screen ---\n${last}`)
}
const rows = (text) => text.split('\n').map((l) => l.trimEnd()).filter(Boolean)

const home = path.join(root, 'home')
const workspace = path.join(root, 'workspace')
for (const dir of [home, workspace, path.join(root, 'state'), path.join(root, 'config'), path.join(root, 'tmp')]) mkdirSync(dir, { recursive: true })
writeFileSync(path.join(home, '.bash_profile'), "alias termalias='echo PROFILE-ALIAS-OK'\n")
const port = 50000 + Math.floor(Math.random() * 1000)
const socket = path.join(root, 'kernel.sock')
const env = {
  PATH: process.env.PATH, HOME: home, SHELL: '/bin/bash', TERM: 'xterm-256color', TMPDIR: path.join(root, 'tmp'),
  XDG_CONFIG_HOME: path.join(root, 'config'), XDG_STATE_HOME: path.join(root, 'state'), CHARIOX_HOME: path.join(root, 'chariox'),
  CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(port + 1000), CHARIOX_OPENCODE_PORT: String(port + 2000),
  CHARIOX_CODEX_PORT: String(port + 2001), CHARIOX_DAEMON_ID: `terminal-attach-${process.pid}`, CHARIOX_DAEMON_SOCKET: socket,
  CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
}
let kernel = null
let client = null
try {
  kernel = spawn(kernelBin, [], { env, stdio: ['ignore', 'ignore', 'inherit'] })
  const kernelUrl = `ws://127.0.0.1:${port}`
  for (let attempt = 0; !client; attempt++) {
    const probe = new LocalIpcClient(kernelUrl, { localAuthEnvironment: env })
    try { await probe.send(requests.listSessionsRequest()); client = probe } catch (error) {
      await probe.close().catch(() => {})
      if (attempt > 80) throw error
      await sleep(250)
    }
  }
  const sessionId = (await client.send(requests.createSessionRequest(workspace, workspace))).SessionCreated.session.id
  const terminals = async () => (await client.send(requests.listRoomTerminalsRequest({ sessionId }))).RoomTerminal.result.terminals
  const kernelScreen = async (terminalId) =>
    (await client.send(requests.roomTerminalSnapshotRequest({ sessionId }, terminalId))).RoomTerminal.result.snapshot.screen_text

  const cliDist = path.join(repoRoot, 'apps/cli/dist/index.js')
  const attach = `bun ${cliDist} terminal --session ${sessionId} --kernel-url ${kernelUrl}`

  // Scenario 1: `chariox terminal` from a shell.
  const pane = 'standalone'
  tmux('new-session', '-d', '-s', pane, '-x', '120', '-y', '36', ...Object.entries(env).flatMap(([k, v]) => ['-e', `${k}=${v}`]), 'bash --norc --noprofile -i')
  line(pane, "PS1='outer$ '")
  await waitFor(pane, (s) => s.includes('outer$'), 'the outer prompt')
  line(pane, 'echo OUTER-BEFORE-$((20+22))')
  await waitFor(pane, (s) => s.includes('OUTER-BEFORE-42'), 'the outer echo')

  line(pane, attach)
  await sleep(2500)
  line(pane, 'termalias; echo hello-$((6*7)); pwd')
  await waitFor(pane, (s) => s.includes('PROFILE-ALIAS-OK') && s.includes('hello-42') && s.includes(workspace), 'the login shell in the project folder')
  keep('1-attached-shell', pane)
  check(true, 'chariox terminal opens a login shell in the project folder (profile alias works)')

  line(pane, 'python3')
  await waitFor(pane, (s) => s.includes('>>>'), 'the python prompt')
  line(pane, 'print(6*7)')
  await waitFor(pane, (s) => /^42$/m.test(s), 'the python output')
  key(pane, 'C-]')
  let outerScreen = await waitFor(pane, (s) => s.includes('Detached from terminal'), 'the detach message')
  keep('2-detached', pane)
  check(outerScreen.includes('OUTER-BEFORE-42') && outerScreen.includes('outer$'), 'Ctrl-] returns to the outer shell with its earlier content intact')
  const listed = await terminals()
  check(listed.length === 1 && listed[0].status.state === 'running', 'the terminal keeps running after the detach')
  const terminalId = listed[0].terminal_id
  line(pane, 'echo OUTER-AFTER-$((5*5))')
  await waitFor(pane, (s) => s.includes('OUTER-AFTER-25'), 'the outer shell after the detach')

  const pythonScreen = rows(await kernelScreen(terminalId))
  line(pane, attach)
  const reattached = rows(await waitFor(pane, (s) => s.includes('print(6*7)') && /^42$/m.test(s), 're-attached python screen'))
  keep('3-reattached-python', pane)
  check(pythonScreen.every((l) => reattached.includes(l)), 're-attach shows the screen the kernel holds (python session)')

  line(pane, 'exit()')
  line(pane, 'vim -u NONE -N')
  await waitFor(pane, (s) => /^~/m.test(s), 'vim')
  type(pane, 'ihello-from-vim')
  await sleep(400)
  key(pane, 'C-]')
  outerScreen = await waitFor(pane, (s) => (s.match(/Detached from terminal/g) ?? []).length >= 2, 'the second detach message')
  keep('4-vim-detached', pane)
  check(outerScreen.includes('OUTER-AFTER-25') && !/^~$/m.test(outerScreen), "leaving vim's alternate screen restores the outer shell screen")
  const vimScreen = rows(await kernelScreen(terminalId))
  check(vimScreen.some((l) => l.includes('hello-from-vim')), 'the kernel holds the vim screen')
  line(pane, attach)
  const vimAgain = rows(await waitFor(pane, (s) => s.includes('hello-from-vim') && /^~/m.test(s) && s.includes('INSERT'), 're-attached vim screen'))
  keep('5-reattached-vim', pane)
  check(vimScreen.every((l) => vimAgain.includes(l)), 're-attach shows the same vim screen')
  key(pane, 'Escape')
  line(pane, ':q!')
  await sleep(500)
  line(pane, 'exit')
  outerScreen = await waitFor(pane, (s) => /ended: exited with code 0/.test(s), 'the end of the shell')
  keep('6-ended', pane)
  check(true, 'when the shell exits the attach ends and says why')

  // Scenario 2: the TUI's `/room terminal`.
  const tui = 'tui'
  tmux('new-session', '-d', '-s', tui, '-x', '140', '-y', '44', ...Object.entries(env).flatMap(([k, v]) => ['-e', `${k}=${v}`]),
    `bun ${cliDist} --kernel-url ${kernelUrl} --session ${sessionId} --workspace ${workspace} --worktree ${workspace}`)
  await waitFor(tui, (s) => s.trim().length > 0, 'the TUI', 40_000)
  await sleep(3000)
  line(tui, '/room terminal list')
  const before = await waitFor(tui, (s) => s.includes('No terminals in this Room') || /\d\. term/.test(s), 'the terminal list in the TUI')
  keep('7-tui-before', tui)
  line(tui, '/room terminal new tui-tab')
  await waitFor(tui, (s) => s.includes(workspace) && !s.includes('/room terminal new'), 'the terminal in place of the TUI')
  await sleep(1500)
  line(tui, 'echo in-tui-$((6*7))')
  await waitFor(tui, (s) => s.includes('in-tui-42'), 'shell output inside the TUI attach')
  line(tui, 'python3')
  await waitFor(tui, (s) => s.includes('>>>'), 'python in the TUI attach')
  line(tui, 'print(7*7)')
  await waitFor(tui, (s) => /^49$/m.test(s), 'python output in the TUI attach')
  keep('8-tui-attached-python', tui)
  key(tui, 'C-]')
  const after = await waitFor(tui, (s) => s.includes('Detached from terminal') || s.includes('/room terminal list'), 'the TUI back')
  keep('9-tui-after', tui)
  check(rows(before).filter((l) => l.includes('No terminals in this Room') || /\d\. term/.test(l)).every((l) => after.includes(l.trim())),
    'back in the TUI its transcript is intact')
  check(!after.includes('in-tui-42') && !after.includes('>>>'), "the terminal's screen is gone and the TUI is drawn again")
  line(tui, '/room terminal list')
  await waitFor(tui, (s) => /\d\. term\S+\s+tui-tab/.test(s), 'the TUI lists the running terminal')
  line(tui, '/room terminal')
  const reattachedTui = await waitFor(tui, (s) => s.includes('print(7*7)') && /^49$/m.test(s) && s.includes('>>>'), 'the TUI re-attach shows python again')
  keep('10-tui-reattached', tui)
  check(reattachedTui.includes('print(7*7)'), 'TUI re-attach shows the same python screen')
  key(tui, 'C-]')
  await waitFor(tui, (s) => !s.includes('>>>'), 'the TUI back again')
  line(tui, '/room terminal list')
  await sleep(500)
  check(/term\S+\s+tui-tab/.test(screen(tui)), 'the TUI still answers commands after two attaches')
} catch (error) {
  check(false, `drill error: ${error.stack ?? error}`)
} finally {
  try { tmux('kill-server') } catch { /* no server */ }
  await client?.close().catch(() => {})
  if (kernel?.pid && kernel.pid > 1) { try { process.kill(kernel.pid, 'SIGTERM') } catch { /* gone */ } }
  await sleep(1500)
  if (!process.env.DRILL_KEEP) rmSync(root, { recursive: true, force: true })
}
const failed = results.filter((r) => !r.ok)
console.log(failed.length === 0 ? `RESULT PASS (${results.length})` : `RESULT FAIL (${failed.length} of ${results.length})`)
process.exit(failed.length === 0 ? 0 : 1)
