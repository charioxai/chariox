#!/usr/bin/env bun
// MP-08/MP-11: real built TUI + PTY + xterm terminal. Login payloads ONLY are
// fixtures: never run real provider login/logout or touch a shared account.
// --attached yes drives a real provider session on an existing kernel
// (--fleet-home/--fleet-kernel-url/--account-profile/--model) instead.
import assert from 'node:assert/strict'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, readFile, writeFile, rm, mkdtemp, readdir } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { WebSocketServer } from 'ws'

const hashClient = async root => {
  const hash = createHash('sha256')
  const visit = async (directory, prefix = '') => {
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a,b) => a.name.localeCompare(b.name))) {
      if (entry.isDirectory()) await visit(path.join(directory,entry.name), `${prefix}${entry.name}/`)
      else if (entry.isFile()) { hash.update(`${prefix}${entry.name}\0`); hash.update(await readFile(path.join(directory,entry.name))) }
    }
  }
  await visit(root)
  return hash.digest('hex')
}
const hashFile = async filename => {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(filename)) hash.update(chunk)
  return hash.digest('hex')
}
const options = Object.fromEntries(process.argv.slice(2).reduce((pairs, item, i, args) => i % 2 ? pairs : [...pairs, [item.replace(/^--/, ''), args[i + 1]]], []))
const cli = path.resolve(options.cli ?? 'apps/cli/dist/index.js')
const evidence = path.resolve(options.output)
const tools = options.tools ? path.resolve(options.tools) : null
assert.ok(!evidence.startsWith(`${process.cwd()}/`), 'evidence must be outside the repository')
assert.ok(process.env.TMPDIR && !process.env.TMPDIR.startsWith('/tmp'), 'TMPDIR must be on disk')
const chromium = options.interactive ? null : createRequire(path.join(tools, 'package.json'))('playwright').chromium
const scratch = await mkdtemp(path.join(process.env.TMPDIR, 'tuifix-'))
const runtimeEnv = { ...process.env }
for (const key of Object.keys(runtimeEnv)) if (key.startsWith('CHARIOX_')) delete runtimeEnv[key]
Object.assign(runtimeEnv, {
  HOME: path.join(scratch,'home'), XDG_CONFIG_HOME: path.join(scratch,'xdg-config'),
  XDG_STATE_HOME: path.join(scratch,'xdg-state'), XDG_DATA_HOME: path.join(scratch,'xdg-data'),
  CODEX_HOME: path.join(scratch,'codex'), CLAUDE_CONFIG_DIR: path.join(scratch,'claude'),
  OPENCODE_CONFIG_DIR: path.join(scratch,'opencode'),
  // Separate client discovery state prevents bypassing the fixture proxy.
  CHARIOX_HOME: path.join(scratch,'client-state'), CHARIOX_LOG_DIR: path.join(scratch,'client-logs'),
  ...(process.env.CHARIOX_TUI_MOUSE === 'off' ? { CHARIOX_TUI_MOUSE: 'off' } : {}),
})
await mkdir(runtimeEnv.HOME, { recursive: true })
await mkdir(evidence, { recursive: true })
const url = 'https://claude.ai/oauth/authorize?client_id=fixture&redirect_uri=https%3A%2F%2Fexample.org%2Fcallback&state=' + '0123456789abcdef'.repeat(25)
const deviceUrl = 'https://auth.openai.com/codex/device'
const requests = []
const upstreamResponses = []
let output = '', tui, browser, page, frontend, kernel
const clients = new Set()
const fixture = new WebSocketServer({ port: 0, host: '127.0.0.1' })
await new Promise(resolve => fixture.once('listening', resolve))
let kernelUrl
let KernelClient
const upstreamClients = new Set()
let result
const sessionAlias = `tuifix-drill-${process.pid}`
let sessionDeleted = true
const deleteSession = async () => {
  const client = new KernelClient(options['fleet-kernel-url'], {})
  try { await client.send({ DeleteSession: { session_ref: sessionAlias, workspace_id: null } }); sessionDeleted = true }
  finally { await client.close() }
}
// Kernel-side status of the drill's queued prompt (authoritative "not cancelled").
const queuedPromptStatus = async prompt => {
  const client = new KernelClient(options['fleet-kernel-url'], {})
  try {
    const resolved = JSON.stringify(await client.send({ ResolveSession: { session_ref: sessionAlias, workspace_id: null } }))
    const sessionId = resolved.match(/"session_id":"([^"]+)"/)?.[1]
    const state = JSON.stringify(await client.send({ GetSessionState: { session_id: sessionId } }))
    return state.match(new RegExp(`"prompt":"${prompt}[^"]*"[^}]*"status":"([A-Za-z]+)"`))?.[1] ?? 'absent'
  } finally { await client.close() }
}
const stop = async child => {
  if (!child) return
  if (child.exitCode === null) {
    assert.ok(Number.isInteger(child.pid) && child.pid > 1, 'reject unsafe PID')
    child.kill('SIGTERM')
    await Promise.race([child.exited, Bun.sleep(3000)])
    if (child.exitCode === null) child.kill('SIGKILL')
  }
  await child.exited
  child.terminal?.close()
}
const sleep = ms => Bun.sleep(ms)
const waitFor = async (predicate, timeoutMs = 20_000) => {
  const deadline = Date.now() + timeoutMs
  while (!await predicate()) { if (Date.now() > deadline) throw Error('terminal condition timed out'); await sleep(50) }
}
try {
  if (options['kernel-binary']) {
    // Own exact-source kernel. The proxy replaces login responses and forwards
    // ordinary inventory/control traffic without logging headers or payloads.
    const listener = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } })
    const mcpListener = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } })
    const mcpPort = mcpListener.port; mcpListener.stop(true)
    const port = listener.port; listener.stop(true)
    kernelUrl = `ws://127.0.0.1:${port}/kernel`
    process.env.CHARIOX_HOME = path.join(scratch, 'state')
    KernelClient = (await import(path.join(path.dirname(cli), 'ipc.js'))).LocalIpcClient
    kernel = Bun.spawn([path.resolve(options['kernel-binary'])], { env: { ...runtimeEnv, CHARIOX_HOME: path.join(scratch, 'state'), CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_DAEMON_SOCKET: path.join(scratch,'kernel.sock'), CHARIOX_LOG_DIR: path.join(scratch, 'logs') }, stdout: 'ignore', stderr: 'ignore' })
    const probe = new KernelClient(kernelUrl, {})
    upstreamClients.add(probe)
    // The kernel serves WebSocket RPCs, not an HTTP /health endpoint.
    await waitFor(async () => {
      if (kernel.exitCode !== null) throw Error(`owned kernel exited: ${kernel.exitCode}`)
      try { return Object.keys(await probe.send({ RelayStatus: null }))[0] === 'RelayStatus' } catch { return false }
    })
    if (options.attached) {
      assert.ok(options['profile-path'], 'owned real-provider kernel requires an existing product-linked profile path')
      const linked = await probe.send({ LinkProviderAccountProfile: {
        provider: options.provider ?? 'codex', label: 'tuifix-native-copy', path: path.resolve(options['profile-path']),
      } })
      const profileId = linked.ProviderAccountProfile?.profile?.profile_id
      assert.ok(profileId, 'product account linking must succeed')
      // MP-08 / MP-10: linking records an unknown auth observation. Use the
      // documented product status refresh before prompt admission; never run
      // an official login or mutate shared credentials in the drill.
      const refreshed = await probe.send({ RefreshProviderAccountProfile: {
        provider: options.provider ?? 'codex', account_profile: profileId,
      } })
      const authState = refreshed.ProviderAccountProfile?.profile?.auth_state
      await writeFile(path.join(evidence, 'linked-profile-status.json'), JSON.stringify({
        items: ['MP-08','MP-10'], linkedAuthState: linked.ProviderAccountProfile.profile.auth_state,
        refreshedAuthState: authState,
      }, null, 2))
      assert.equal(authState, 'authenticated', 'product-linked profile must be verified before prompt admission')
      options['account-profile'] = profileId
      options['fleet-home'] = path.join(scratch, 'state')
      options['fleet-kernel-url'] = kernelUrl
    }
    await probe.close()
  }
  if (options.attached) {
    // Normal product client authorization from the account-holding kernel home.
    process.env.CHARIOX_HOME = path.resolve(options['fleet-home'])
    KernelClient = (await import(path.join(path.dirname(cli), 'ipc.js'))).LocalIpcClient
  }
  fixture.on('connection', (socket) => {
    const upstream = kernelUrl ? new KernelClient(kernelUrl, {}) : null
    if (upstream) {
      upstreamClients.add(upstream)
      upstream.onKernelEvent(event => {
        if (socket.readyState === 1) socket.send(JSON.stringify({ type: 'event', event_id: event.event_id ?? 1, event }))
      })
    }
    socket.on('close', () => { void upstream?.close() })
    socket.on('message', data => {
      const f = JSON.parse(String(data)); const req = f.request ?? {}; const variant = Object.keys(req)[0] ?? f.type
      requests.push(variant)
      let response
      if (variant === 'StartProviderLogin') response = { ProviderLoginStarted: { login: { provider: 'codex', account_profile: 'default', login_kind: 'device_code', login_id: null, verification_url: deviceUrl, auth_url: null, user_code: 'FIXTURE-CODE' } } }
      if (variant === 'GetProviderLoginStatus') response = { ProviderLoginStatus: { login: { provider: 'claude', account_profile: 'default', login_id: 'fixture', state: 'running', terminal_output_base64: Buffer.from('Authorize:\n' + url + '\n').toString('base64'), started_at_ms: 1, updated_at_ms: 1 } } }
      if (!upstream && variant === 'ListProviderAccountProfiles') response = { ProviderAccountProfilesListed: { profiles: [] } }
      if (response) socket.send(JSON.stringify({ type: 'response', request_id: f.request_id, response, error: null }))
      else if (upstream) {
        const request = f.type === 'subscribe' || f.type === 'unsubscribe'
          ? { __kernel_transport: { type: f.type, session_id: f.session_id, attachment_id: f.attachment_id, subscription_scope: f.subscription_scope, resume_from_event_id: f.resume_from_event_id } }
          : req
        void upstream.send(request).then(response => {
          upstreamResponses.push({ request: variant, response: Object.keys(response)[0] })
          if (socket.readyState === 1) socket.send(JSON.stringify({ type: 'response', request_id: f.request_id, response, error: null }))
        }).catch(error => {
          upstreamResponses.push({ request: variant, response: 'transport_error', code: error?.code ?? error?.name, unknownVariant: /unknown variant/.test(error?.message ?? '') })
          if (socket.readyState === 1) socket.send(JSON.stringify({ type: 'response', request_id: f.request_id, response: null, error: { message: 'owned kernel request unavailable', code: error?.code ?? 'owned_kernel_transport_failed', retryable: error?.retryable ?? false } }))
        })
      }
      else socket.send(JSON.stringify({ type: 'response', request_id: f.request_id, response: { Error: { message: 'unsupported fixture request' } }, error: null }))
    })
  })
  if (options.interactive) {
    tui = Bun.spawn(['bun', cli, '--detached', '--kernel-url', `ws://127.0.0.1:${fixture.address().port}/kernel`], {
      cwd: process.cwd(), env: runtimeEnv,
      stdin: 'inherit', stdout: 'inherit', stderr: 'inherit',
    })
    await tui.exited
    result = { items: ['MP-08','MP-11'], source: options.source, cli, cliSha256: await hashClient(path.dirname(cli)), requests, interactive: true, acceptance: 'manual macOS/SSH observations must be recorded by the operator' }
  } else {
  frontend = Bun.serve({ hostname: '127.0.0.1', port: 0,
    fetch(request, server) {
      const route = new URL(request.url).pathname
      if (route === '/terminal' && server.upgrade(request)) return
      if (route === '/xterm.js') return new Response(Bun.file(path.join(tools, 'node_modules/@xterm/xterm/lib/xterm.js')))
      if (route === '/xterm.css') return new Response(Bun.file(path.join(tools, 'node_modules/@xterm/xterm/css/xterm.css')))
      return new Response(`<!doctype html><link rel="stylesheet" href="/xterm.css"><style>body{background:#141414;margin:16px}</style><div id="terminal"></div><script src="/xterm.js"></script><script>
        window.term=new Terminal({cols:100,rows:35,fontSize:16,fontFamily:'monospace',allowProposedApi:true,scrollback:10000});term.open(document.querySelector('#terminal'));term.focus();
        // The terminal host owns its native Copy shortcut, like Cmd-C in Terminal.app.
        term.attachCustomKeyEventHandler(event=>!(event.ctrlKey && event.key==='c' && term.hasSelection()));
        window.openedLinks=[];term.options.linkHandler={activate:(event,text)=>openedLinks.push(text)};
        window.copies=[];term.parser.registerOscHandler(52,data=>{copies.push(data.split(';').slice(1).join(';'));return true});
        window.ws=new WebSocket('ws://'+location.host+'/terminal');ws.onmessage=e=>term.write(e.data);term.onData(data=>ws.send(data));
        window.terminalScreen=()=>Array.from({length:term.rows},(_,i)=>term.buffer.active.getLine(term.buffer.active.viewportY+i)?.translateToString(true)??'').join('\\n');
      </script>`, { headers: { 'content-type': 'text/html' } })
    }, websocket: {
      open(socket) { clients.add(socket); socket.send(output) },
      close(socket) { clients.delete(socket) },
      message(_socket, data) { tui?.terminal.write(String(data)) },
    },
  })
  const tuiArgs = options['waiting-room-paste']
    ? ['--kernel-url', kernelUrl]
    : options.attached
    ? ['--kernel-url', options['fleet-kernel-url'], '--create-session', '--alias', sessionAlias, '--workspace', process.cwd(), '--worktree', process.cwd(),
      '--provider', options.provider ?? 'codex', '--account-profile', options['account-profile'], '--model', options.model]
    : ['--detached', '--kernel-url', `ws://127.0.0.1:${fixture.address().port}/kernel`]
  if (options.attached) sessionDeleted = false
  tui = Bun.spawn(['bun', cli, ...tuiArgs], {
    cwd: process.cwd(), env: { ...runtimeEnv, TERM: options.term ?? 'xterm-256color', SSH_CONNECTION: 'fixture 1 fixture 2',
      ...(options['no-mouse'] ? { CHARIOX_TUI_MOUSE: 'off' } : {}),
      ...(options.attached || options['waiting-room-paste'] ? { CHARIOX_HOME: process.env.CHARIOX_HOME } : {}) },
    terminal: { cols: 100, rows: 35, data(_terminal, chunk) { const data = new TextDecoder().decode(chunk); output += data; for (const c of clients) c.send(data) } },
  })
  browser = await chromium.launch({ headless: true, args: ['--no-sandbox'], executablePath: options.chromium })
  page = await browser.newPage({ viewport: { width: 1050, height: 740 }, deviceScaleFactor: Number(options.dpr ?? 1) })
  await page.goto(`http://127.0.0.1:${frontend.port}`)
  const capture = async name => {
    await sleep(250)
    await page.screenshot({ path: path.join(evidence, `${name}.png`) })
    await writeFile(path.join(evidence, `${name}.txt`), await page.evaluate(() => terminalScreen()))
  }
  const press = async sequence => { tui.terminal.write(sequence); await sleep(180) }
  // MP-08 / MP-10 --fragment-mouse: split every SGR report right after ESC
  // and inside its parameters across PTY writes, as stdin may deliver it.
  const mouse = async sequence => {
    if (!options['fragment-mouse']) return press(sequence)
    const pieces = ['']
    for (const report of sequence.match(/\x1b\[<[^Mm]*[Mm]/g)) {
      const cut = report.indexOf(';') + 2
      pieces[pieces.length - 1] += report[0]
      pieces.push(report.slice(1, cut), report.slice(cut))
    }
    for (const piece of pieces) { tui.terminal.write(piece); await sleep(2) }
    await sleep(180)
  }
  const typeText = async text => { for (const c of text) { tui.terminal.write(c); await sleep(15) } }
  const rowOf = needle => page.evaluate(n => {
    const rows = terminalScreen().split('\n'); const y = rows.findIndex(row => row.includes(n))
    return y < 0 ? null : { x: rows[y].indexOf(n), y }
  }, needle)
  const cellColors = ({ x, y }, width) => page.evaluate(({ x, y, width }) => {
    const line = term.buffer.active.getLine(term.buffer.active.viewportY+y)
    return Array.from({ length: width }, (_, i) => line.getCell(x+i).getBgColor())
  }, { x, y, width })
  const copiedTexts = async () => (await page.evaluate(() => copies)).map(payload => Buffer.from(payload, 'base64').toString())
  const copySequence = options['copy-key'] === 'kitty' ? '\x1b[99;6u' : '\x1b[17~'
  // Real SGR drag across `width` cells of a visible row, then release.
  const dragSelect = async (at, width) => {
    await mouse(`\x1b[<0;${at.x+1};${at.y+1}M`)
    const move = `\x1b[<32;${at.x+width};${at.y+1}M`
    await mouse(options['batch-mouse'] ? `\x1b[<32;${at.x+2};${at.y+1}M${move}` : move)
    await mouse(`\x1b[<0;${at.x+width};${at.y+1}m`)
    await sleep(400)
  }
  const settledRowOf = async needle => {
    // Wait for two identical screens so a collapsing entry cannot shift the drag.
    let previous = ''
    await waitFor(async () => { const now = await page.evaluate(() => terminalScreen()); const same = now === previous; previous = now; await sleep(400); return same }, 30_000)
    return rowOf(needle)
  }
  // MP-08 / MP-10: terminal dark/light reports are unsolicited stdin input.
  // Keep the actual TUI highlight and copy target, including a coalesced F6.
  const themeCases = async (needle, at, before) => {
    const cells = []
    for (const mode of [1, 2]) for (const batched of [false, true]) {
      await dragSelect(at, 14)
      const selected = await cellColors(at, 13)
      const highlighted = JSON.stringify(selected) !== JSON.stringify(before)
      const copiesBefore = (await copiedTexts()).length
      await press(`\x1b[?997;${mode}n` + (batched ? copySequence : ''))
      const retained = JSON.stringify(await cellColors(at, 13)) === JSON.stringify(selected)
      await capture(`theme-${mode}-${batched ? 'batched' : 'separate'}`)
      if (!batched) await press(copySequence)
      const copied = (await copiedTexts()).slice(copiesBefore).some(text => text.startsWith(needle.slice(0, 13)))
      cells.push({ mode, batched, highlighted, retained, copied })
    }
    await writeFile(path.join(evidence, 'theme-notifications.json'), JSON.stringify({ items: ['MP-08', 'MP-10'], cells }, null, 2))
    return cells
  }
  const promptShows = text => page.evaluate(t => terminalScreen().split('\n').slice(-8).some(row => row.includes(t)), text)
  // MP-08 / MP-10: keyboard edits in the real focused prompt, after a real turn.
  const promptKeyboardSelectionCases = async () => {
    const cells = []
    for (const batched of [false, true]) for (const edit of ["later", "typing", "paste", "extend"]) for (const [kind, keys, selected, originalReplacement] of [
      ['left', ['\x1b[1;2D', '\x1b[1;2D'], 'ft', 'draZ'],
      ['home', ['\x1b[1;2H'], 'draft', 'Z'],
    ]) {
      // Reset the whole draft even on RED, where replacement leaves a suffix.
      await press('\x1b[F')
      await press('\x15')
      await typeText('draft')
      await sleep(250)
      assert.ok(await promptShows('draft'), 'keyboard selection precondition: focused draft')
      const count = (await copiedTexts()).length
      await capture(`prompt-${kind}-${batched}-${edit}-before`)
      const suffix = edit === 'typing' ? 'Z' : edit === 'paste' ? '\x1b[200~Z\x1b[201~'
        : edit === 'extend' ? (kind === 'left' ? '\x1b[1;2D' : '\x1b[1;2C') : ''
      const replacement = edit === 'extend' ? (kind === 'left' ? 'drZ' : 'dZ') : originalReplacement
      if (batched) await press(keys.join('') + copySequence + suffix)
      else { for (const key of keys) await press(key); await press(copySequence + suffix) }
      await capture(`prompt-${kind}-${batched}-${edit}-selected`)
      const copied = (await copiedTexts()).slice(count).includes(selected)
      if (edit === 'later' || edit === 'extend') await typeText('Z')
      await sleep(250)
      const replaced = await promptShows(replacement) && !await promptShows('draft') && !await promptShows(replacement + 'ft')
      await capture(`prompt-${kind}-${batched}-${edit}-replaced`)
      cells.push({ kind, batched, edit, selected, replacement, copied, replaced })
      await writeFile(path.join(evidence, 'prompt-selections.json'), JSON.stringify(cells, null, 2))
    }
    await press('\x15')
    return cells
  }
  // MP-08 / MP-10: one PTY write per toggle/input pair, as SSH can buffer
  // them. A populated focused prompt must survive entry and accept exit text.
  const nativeBatchCases = async () => {
    const cells = []
    for (const [kind, sequence] of [
      ['enter', '\r'], ['text', 'BLOCKEDTEXT'],
      ['paste', '\x1b[200~BLOCKEDPASTE\x1b[201~'],
    ]) {
      await press('\x15')
      const promptAt = await rowOf('Write your next prompt here')
      if (promptAt) {
        await mouse(`\x1b[<0;${promptAt.x+3};${promptAt.y+1}M`)
        await mouse(`\x1b[<0;${promptAt.x+3};${promptAt.y+1}m`)
      }
      const draft = 'Reply only with BATCH_UNEXPECTED'
      await typeText(draft)
      await capture(`batch-${kind}-before`)
      assert.ok(await promptShows(draft), 'coalesced-input precondition: populated focused prompt')
      await press('\x1b[18~' + sequence)
      const retained = await promptShows(draft)
      const blocked = !await promptShows('BLOCKED')
      const active = await page.evaluate(() => terminalScreen().includes('Mouse off: drag-select'))
      await capture(`batch-${kind}-entered`)
      await press('\x1b[18~EXITTEXT')
      const exited = !await page.evaluate(() => terminalScreen().includes('Mouse off: drag-select'))
      const accepted = await promptShows(draft + 'EXITTEXT')
      await capture(`batch-${kind}-exited`)
      const cell = {kind, retained, blocked, active, exited, accepted}
      cells.push(cell)
      await writeFile(path.join(evidence, 'native-batch.json'), JSON.stringify(cells, null, 2))
      assert.ok(retained && blocked && active && exited && accepted, 'coalesced F7 input ownership')
    }
    await press('\x15')
    return cells
  }
  // MP-08 / MP-10: the actual TUI must release terminal mouse ownership so
  // native drag + system Copy works without OSC 52, then restore app input.
  const nativeCopyCases = async needle => {
    const at = await settledRowOf(needle)
    assert.ok(at, 'native copy text is visible')
    const mark = output.length
    const oscCopies = (await copiedTexts()).length
    await press('\x1b[18~') // F7: legacy Terminal.app function key
    await waitFor(async () => /\x1b\[\?100[0236]l/.test(output.slice(mark)) && await page.evaluate(() => terminalScreen().includes('Mouse off: drag-select')), 5000)
    const mouseDisabled = /\x1b\[\?100[0236]l/.test(output.slice(mark))
    const hint = await page.evaluate(() => terminalScreen().includes('Mouse off: drag-select, Cmd-C; Esc/F7: mouse on'))
    await capture('native-01-mouse-off')
    const coords = await page.evaluate(({ x, y }) => {
      const rect = document.querySelector('.xterm-screen').getBoundingClientRect()
      return { x: rect.x, y: rect.y, cw: rect.width/term.cols, ch: rect.height/term.rows }
    }, at)
    await page.mouse.move(coords.x+coords.cw*(at.x+0.2), coords.y+coords.ch*(at.y+0.5))
    await page.mouse.down()
    await page.mouse.move(coords.x+coords.cw*(at.x+needle.length+0.2), coords.y+coords.ch*(at.y+0.5), { steps: 20 })
    await page.mouse.up()
    const nativeText = await page.evaluate(() => term.getSelection())
    // Trusted browser Copy acts on xterm's native selection, not a Chariox
    // clipboard RPC or OSC 52. Linux Ctrl-C corresponds to desktop Cmd-C.
    await page.context().grantPermissions(['clipboard-read', 'clipboard-write'])
    await page.keyboard.press('Control+c')
    const clipboardText = await page.evaluate(() => navigator.clipboard.readText())
    await sleep(12_000) // selection and instructions survive inventory refresh
    const retained = await page.evaluate(n => term.getSelection() === n, needle)
    await capture('native-02-copied')
    const restoreMark = output.length
    await press('\x1b') // lone Esc, including parser timeout
    await waitFor(async () => /\x1b\[\?100[0236]h/.test(output.slice(restoreMark)) && !await page.evaluate(() => terminalScreen().includes('Mouse off: drag-select')), 5000)
    const escaped = !await page.evaluate(() => terminalScreen().includes('Mouse off: drag-select'))
    const mouseRestored = /\x1b\[\?100[0236]h/.test(output.slice(restoreMark))
    await capture('native-03-escaped')
    await press('\x1b[18~')
    await waitFor(() => page.evaluate(() => terminalScreen().includes('Mouse off: drag-select')), 5000)
    await press('\x1b[18~')
    await waitFor(() => page.evaluate(() => !terminalScreen().includes('Mouse off: drag-select')), 5000)
    const toggledBack = !await page.evaluate(() => terminalScreen().includes('Mouse off: drag-select'))
    const noOscCopy = (await copiedTexts()).length === oscCopies
    const cell = { mouseDisabled, hint, nativeText, clipboardText, retained, escaped, mouseRestored, toggledBack, noOscCopy }
    console.log(JSON.stringify({ items: ['MP-08', 'MP-10'], nativeCopy: cell }))
    await writeFile(path.join(evidence, 'native-copy.json'), JSON.stringify(cell, null, 2))
    assert.ok(mouseDisabled && hint && nativeText === needle && clipboardText === needle && retained && escaped && mouseRestored && toggledBack && noOscCopy, 'native selection/copy through real terminal input')
    return cell
  }
  // MP-08 / MP-10: send text in one PTY write, matching paste/SSH batching.
  const pasteCases = async (needle, beforePaste) => {
    const cells = []
    for (const [kind, text, sequence] of [
      ['bracketed-paste', 'pasted text', '\x1b[200~pasted text\x1b[201~'],
      ['batched-text', 'ab', 'ab'],
    ]) {
      await press('\x15')
      const at = await settledRowOf(needle)
      assert.ok(at, 'paste selection target visible')
      const before = await cellColors(at, needle.length)
      await dragSelect(at, needle.length + 1)
      const highlighted = JSON.stringify(await cellColors(at, needle.length)) !== JSON.stringify(before)
      await capture(`${kind}-dragged`)
      assert.ok(highlighted, 'paste begins with a retained selection')
      const deferred = await beforePaste?.(kind)
      await capture(`${kind}-selected`)
      await press(sequence)
      await sleep(700)
      const cleared = JSON.stringify(await cellColors(at, needle.length)) === JSON.stringify(before)
      const inserted = await promptShows(text)
      const rebuilt = deferred ? await deferred() : null
      await capture(`${kind}-after`)
      cells.push({kind, highlighted, cleared, inserted, rebuilt})
      // Settle for the next case only AFTER observing paste without a named key.
      await press('\x15')
    }
    return cells
  }
  if (options['waiting-room-paste']) {
    // Real owned kernel inventory changes while the user retains a selection.
    // Empty profiles need no login/credential and vanish with the owned home.
    await waitFor(async () => (await rowOf('Provider Accounts')) !== null, 60_000)
    await sleep(12_000)
    let count = await page.evaluate(() => {
      const row = terminalScreen().split('\n').find(row => row.includes('Provider Accounts'))
      return Number(row.match(/(\d+) profiles/)[1])
    })
    const client = new KernelClient(kernelUrl, {})
    let cells
    try {
      cells = await pasteCases('Workspace', async kind => {
        const response = await client.send({CreateProviderAccountProfile: {provider: 'codex', label: `tuifix-paste-${kind}`}})
        assert.ok(response.ProviderAccountProfile, 'real profile creation')
        count++
        await sleep(12_000)
        const expected = `${count} profiles`
        const inventoryVisible = () => page.evaluate(text => terminalScreen().split('\n').some(row => row.includes('Provider Accounts') && row.includes(text)), expected)
        assert.equal(await inventoryVisible(), false, 'inventory rebuild held during retained selection')
        return inventoryVisible
      })
    } finally { await client.close() }
    result = {items: ['MP-08','MP-10'], mode: 'waiting-room-paste', source: options.source,
      cli, cliSha256: await hashClient(path.dirname(cli)), kernelSha256: await hashFile(options['kernel-binary']),
      dpr: Number(options.dpr ?? 1), fragmentMouse: Boolean(options['fragment-mouse']), cells}
    const green = cells.every(cell => cell.cleared && cell.rebuilt)
    console.log(JSON.stringify(result))
    if (options['expect-red']) assert.ok(!green, 'base must fail paste selection/refresh')
    else assert.ok(green, 'paste clears selection and flushes real waiting-room inventory')
  } else if (options.attached) {
    // MP-08/MP-11 attached transcript: real provider turns, then the user's
    // select/click-then-type, Meta+C with a queued prompt, and deletion elsewhere.
    const marker = 'TUIFIX MARKER SEVEN'
    await waitFor(async () => (await page.evaluate(() => terminalScreen().trim().length)) > 0, 60_000)
    await sleep(8000)
    await capture('a01-attached')
    if (options.provider?.startsWith('claude')) {
      await typeText('/permissions required'); await press('\r'); await sleep(1000)
    }
    await typeText('Reply with exactly one line: the words tuifix marker seven alpha, written in uppercase.')
    await press('\r')
    await capture('a01b-prompt-sent')
    await waitFor(async () => (await rowOf(marker)) !== null, 240_000).catch(async error => { await capture('a01c-response-timeout'); throw error })
    // MP-08 / MP-10: the marker can stream before final turn settlement.
    // Select the completed response; settlement may still replace its view.
    await waitFor(() => page.evaluate(() => terminalScreen().split('\n').slice(-8).some(row => /^\s*[│ ]*IDLE\b/.test(row))), 240_000)
    await sleep(4000)
    await capture('a02-response')
    const nativeCopy = options['native-selection-review'] ? await nativeCopyCases(marker) : null
    if (options['native-batch-review']) {
      const nativeBatch = await nativeBatchCases()
      result = {items: ['MP-08','MP-10'], mode: 'native-batch-review', source: options.source,
        provider: options.provider, model: options.model, dpr: Number(options.dpr ?? 1), nativeBatch, nativeCopy,
        acceptance: 'real provider and built TUI; physical Terminal.app/hosted and soak acceptance require separate observations'}
    } else if (options['selection-review']) {
      // MP-08 / MP-10: real provider response, fast terminal drag, legacy copy
      // input, and typing after release. No fixture provider/login traffic.
      const at = await settledRowOf(marker)
      const before = await cellColors(at, 13)
      const copyCount = (await copiedTexts()).length
      await dragSelect(at, 14)
      const highlighted = JSON.stringify(await cellColors(at, 13)) !== JSON.stringify(before)
        && (await copiedTexts()).slice(copyCount).some(text => text.startsWith('TUIFIX MARKER'))
      await capture('a03-batched-selection')
      await sleep(12_000)
      const copiesBeforeKey = (await copiedTexts()).length
      await press(copySequence)
      const keyboardCopy = (await copiedTexts()).slice(copiesBeforeKey).some(text => text.startsWith('TUIFIX MARKER'))
      await capture('a04-legacy-copy')
      const themes = options['theme-review'] ? await themeCases(marker, at, before) : []
      const pasted = options['paste-review'] ? await pasteCases(marker) : []
      await typeText('zq'); await sleep(500)
      const typedAfterDrag = await promptShows('zq')
      const clearedByTyping = JSON.stringify(await cellColors(at, 13)) === JSON.stringify(before)
      await press('\x15'); await press(copySequence)
      const emptyCopyKeptAlive = tui.exitCode === null
      await capture('a05-typed-and-empty-copy')
      const promptSelections = options['prompt-selection-review'] ? await promptKeyboardSelectionCases() : []
      result = { items: ['MP-08', 'MP-10'], mode: 'selection-review', source: options.source,
        kernelBinary: options['kernel-binary'] ?? null, kernelSha256: options['kernel-binary'] ? await hashFile(options['kernel-binary']) : null,
        cli, cliSha256: await hashClient(path.dirname(cli)), kernelUrl: options['fleet-kernel-url'],
        provider: options.provider ?? 'codex', accountProfile: options['account-profile'], model: options.model,
        dpr: Number(options.dpr ?? 1), batchMouse: Boolean(options['batch-mouse']), fragmentMouse: Boolean(options['fragment-mouse']), copyKey: options['copy-key'] ?? 'f6',
        nativeCopy, highlighted, keyboardCopy, typedAfterDrag, clearedByTyping, emptyCopyKeptAlive, pasted, themes, promptSelections,
        acceptance: 'real provider and built TUI via PTY; native clipboard uses a Linux browser terminal; Terminal.app/SSH and hosted transport need separate observations' }
      const green = highlighted && keyboardCopy && typedAfterDrag && clearedByTyping && emptyCopyKeptAlive
        && promptSelections.every(cell => cell.copied && cell.replaced)
        && pasted.every(cell => cell.cleared && cell.inserted)
        && themes.every(cell => cell.highlighted && cell.retained && cell.copied)
      console.log(JSON.stringify(result))
      if (options['expect-red']) assert.ok(!green, 'baseline must fail selection/copy review')
      else assert.ok(green, 'real-provider batched selection and legacy copy')
    } else {
    const selected = text => text.startsWith('TUIFIX MARKER')
    let at = await rowOf(marker)
    const before = await cellColors(at, 13)
    let copyCount = (await copiedTexts()).length
    await dragSelect(at, 14)
    const highlighted = JSON.stringify(await cellColors(at, 13)) !== JSON.stringify(before)
      && (await copiedTexts()).slice(copyCount).some(selected)
    await capture('a03-selected')
    // An idle attached session must keep the selection across the ~10s
    // waiting-room inventory refresh, and the copy key must still copy it.
    await sleep(12_000)
    copyCount = (await copiedTexts()).length
    await press(copySequence)
    await sleep(500)
    const heldAfterRefresh = JSON.stringify(await cellColors(at, 13)) !== JSON.stringify(before)
      && (await copiedTexts()).slice(copyCount).some(selected)
    await capture('a03b-held-after-refresh')
    await typeText('zq'); await sleep(500)
    const typedAfterDrag = await promptShows('zq')
    const clearedByTyping = JSON.stringify(await cellColors(at, 13)) === JSON.stringify(before)
    await capture('a04-typed-after-drag')
    await press('\x15')
    // Zero-length click on transcript text, then type.
    await mouse(`\x1b[<0;${at.x+3};${at.y+1}M`); await mouse(`\x1b[<0;${at.x+3};${at.y+1}m`); await sleep(400)
    await typeText('zr'); await sleep(500)
    const typedAfterClick = await promptShows('zr')
    await capture('a05-typed-after-click')
    await press('\x15')
    // A user whose keys were lost clicks the prompt before continuing.
    const promptRow = await rowOf('Write your next prompt here')
    if (promptRow) { await mouse(`\x1b[<0;${promptRow.x+3};${promptRow.y+1}M`); await mouse(`\x1b[<0;${promptRow.x+3};${promptRow.y+1}m`); await sleep(300) }
    let selectedWithQueue = false, queuedAtMeta = false, metaCopies = [], metaCopy = false, queueStripAfterMeta = false
    let kernelQueuedAfterMeta = 'not-reached', queuedSurvived = false, waitingRoomAfterDelete = false, flowError = null
    try {
    // A running tool turn plus a queued prompt; Meta+C on a selection must
    // copy, not cancel. The selection is made while the transcript is quiet.
    await typeText('Run the shell command `sleep 40` in the workspace, then reply with only the word: slept.')
    await press('\r')
    await waitFor(async () => (await rowOf('sleep 40')) !== null && (await page.evaluate(() => /\bTHINK|WORKING|RUNNING/.test(terminalScreen()))), 120_000)
    await sleep(6000)
    await typeText('Reply with exactly one line: the words queued ok marker, written in uppercase.')
    await press('\r'); await sleep(2500)
    await capture('a06-queued')
    // Running-tool entries can shift rows; confirm the selection by its
    // release copy and retry a drag that missed.
    for (let attempt = 0; attempt < 4 && !selectedWithQueue; attempt++) {
      at = await settledRowOf(marker)
      copyCount = (await copiedTexts()).length
      await dragSelect(at, 14)
      selectedWithQueue = (await copiedTexts()).slice(copyCount).some(selected)
    }
    queuedAtMeta = !(await rowOf('QUEUED OK MARKER')) && await page.evaluate(() => terminalScreen().includes('QUEUE • 1 prompt'))
    const copiesBeforeMeta = (await copiedTexts()).length
    if (options['meta-key'] !== 'none') await press('\x1bc')
    await sleep(600)
    metaCopies = (await copiedTexts()).slice(copiesBeforeMeta)
    metaCopy = metaCopies.some(selected)
    queueStripAfterMeta = await page.evaluate(() => terminalScreen().includes('QUEUE • 1 prompt'))
    kernelQueuedAfterMeta = await queuedPromptStatus('Reply with exactly one line: the words queued ok marker')
    await capture('a07-meta-c')
    queuedSurvived = true
    try { await waitFor(async () => (await rowOf('QUEUED OK MARKER')) !== null, 300_000) } catch { queuedSurvived = false }
    await sleep(4000)
    await capture('a08-queued-result')
    // Delete the session elsewhere while transcript text is selected.
    at = (await settledRowOf(marker)) ?? (await rowOf('QUEUED OK MARKER'))
    await dragSelect(at, 14)
    await capture('a09-selected-before-delete')
    await deleteSession()
    waitingRoomAfterDelete = true
    // The waiting-room menu itself (transcript content), not just the footer chrome.
    try { await waitFor(() => page.evaluate(() => /┃\s+(Join Existing Session|Projects|Provider Accounts|Machines|Theme)\b/.test(terminalScreen())), 20_000) } catch { waitingRoomAfterDelete = false }
    await capture('a10-after-delete')
    } catch (error) { flowError = error?.message ?? String(error) }
    result = { items: ['MP-08','MP-11'], mode: 'attached', cli, cliSha256: await hashClient(path.dirname(cli)), source: options.source, dpr: Number(options.dpr ?? 1),
      kernelUrl: options['fleet-kernel-url'], provider: options.provider ?? 'codex', accountProfile: options['account-profile'], model: options.model,
      highlighted, heldAfterRefresh, typedAfterDrag, clearedByTyping, typedAfterClick, selectedWithQueue, queuedAtMeta, metaCopy, metaCopies, queueStripAfterMeta, kernelQueuedAfterMeta, queuedSurvived, waitingRoomAfterDelete, flowError,
      acceptance: 'real provider turns on the shared fleet kernel; macOS Terminal.app behavior requires the coordinator desktop check' }
    await writeFile(path.join(evidence, 'terminal.pty'), output)
    console.log(JSON.stringify(result))
    if (!options['expect-red']) assert.ok(selectedWithQueue && queuedAtMeta, 'Meta+C precondition: a selection while a prompt is queued')
    const green = !flowError && highlighted && heldAfterRefresh && typedAfterDrag && clearedByTyping && typedAfterClick && metaCopy && queueStripAfterMeta && kernelQueuedAfterMeta === 'Queued' && waitingRoomAfterDelete
    if (options['expect-red']) assert.ok(!green, 'baseline must fail')
    else assert.ok(green, 'attached selection/focus regression')
    }
  } else {
  await waitFor(() => page.evaluate(() => terminalScreen().includes('Provider Accounts')))
  const command = async text => {
    // The ordinary waiting-room account control stages and focuses a command.
    if (options['no-mouse']) { await press('\x1b[A'); await press('\x1b[A') }
    else {
    for (let i = 0; i < 60; i++) {
      // The completed intro pushes the bottom control rows below a short PTY.
      tui.terminal.write('\x1b[<65;60;20M'.repeat(12)); await sleep(100)
      if (await page.evaluate(() => />\s+Provider Accounts/.test(terminalScreen()))) break
      await press('\x1b[A')
    }
    assert.ok(await page.evaluate(() => />\s+Provider Accounts/.test(terminalScreen())), 'navigate to the visible Provider Accounts control')
    }
    await press('\r')
    await waitFor(() => page.evaluate(() => terminalScreen().includes('/provider accounts')), 90_000)
    await capture('03b-accounts-command'); await press('\x15')
    for (const c of text) { tui.terminal.write(c); await sleep(15) }
    await press('\r')
  }
  await capture('01-waiting-room')
  let nativeCopy = null
  if (options['native-selection-review']) nativeCopy = await nativeCopyCases('Provider Accounts')
  // Select an ordinary text row via actual SGR mouse reports, then release.
  // Compare the terminal's cell colors while dragging and after release.
  const selection = await page.evaluate(() => {
    const rows = terminalScreen().split('\n'); const y = rows.findIndex(row => row.includes('Provider Accounts'))
    return { x: rows[y].indexOf('Provider Accounts'), y }
  })
  const colors = () => page.evaluate(({x,y}) => {
    const line = term.buffer.active.getLine(term.buffer.active.viewportY+y)
    return Array.from({length:16},(_,i)=>line.getCell(x+i).getBgColor())
  }, selection)
  const before = await colors()
  let retained
  if (options['no-mouse']) {
    const coords = await page.evaluate(({x,y}) => {
      const rect = document.querySelector('.xterm-screen').getBoundingClientRect()
      return { x: rect.x, y: rect.y, cw: rect.width/term.cols, ch: rect.height/term.rows, col: x, row: y }
    }, selection)
    await page.mouse.move(coords.x+coords.cw*(coords.col+0.2), coords.y+coords.ch*(coords.row+0.5))
    await page.mouse.down()
    await page.mouse.move(coords.x+coords.cw*(coords.col+17.2), coords.y+coords.ch*(coords.row+0.5), { steps: 20 })
    await capture('02-dragging')
    await page.mouse.up()
    await sleep(500)
    retained = await page.evaluate(() => term.getSelection() === 'Provider Accounts')
  } else {
  await mouse(`\x1b[<0;${selection.x+1};${selection.y+1}M`)
  const move = `\x1b[<32;${selection.x+16};${selection.y+1}M`
  await mouse(options['batch-mouse'] ? `\x1b[<32;${selection.x+8};${selection.y+1}M${move}` : move)
  }
  const during = await colors()
  if (!options['no-mouse']) await capture('02-dragging')
  const copyMark = output.length
  if (!options['no-mouse']) await mouse(`\x1b[<0;${selection.x+16};${selection.y+1}m`)
  await sleep(500)
  const after = await colors()
  await capture('03-released')
  if (!options['no-mouse']) retained = JSON.stringify(before) !== JSON.stringify(during) && JSON.stringify(during) === JSON.stringify(after)
  const copyCount = await page.evaluate(() => copies.length)
  if (!options['no-mouse']) await press(copySequence)
  const keyboardCopy = options['no-mouse'] ? null : await page.evaluate(count => copies.length > count, copyCount)
  const themes = options['theme-review'] ? await themeCases('Provider Accounts', selection, before.slice(0, 13)) : []
  // Stage a command through real key input. Never start a real login process.
  await command('/provider login-status fixture')
  await waitFor(() => requests.includes('GetProviderLoginStatus'))
  await sleep(600)
  await capture('04-claude-link')
  let nativeSelection = false
  let hyperlinkActivated = false
  if (output.includes(`\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\`)) {
    const coords = await page.evaluate(length => {
      const y = terminalScreen().split('\n').findIndex(line => line.startsWith('https://claude.ai/'))
      const rect = document.querySelector('.xterm-screen').getBoundingClientRect()
      return { x: rect.x, y: rect.y, cw: rect.width/term.cols, ch: rect.height/term.rows, row: y, endRow: y+Math.floor(length/term.cols), endCol: length%term.cols }
    }, url.length)
    assert.ok(coords.row >= 0)
    await page.mouse.move(coords.x+coords.cw*0.2, coords.y+coords.ch*(coords.row+0.5))
    await page.mouse.down()
    await page.mouse.move(coords.x+coords.cw*(coords.endCol+0.2), coords.y+coords.ch*(coords.endRow+0.5), { steps: 20 })
    await page.mouse.up()
    nativeSelection = await page.evaluate(expected => term.getSelection() === expected, url)
    await capture('04b-native-url-selection')
    await page.keyboard.down('Control')
    await page.mouse.move(coords.x+coords.cw*10, coords.y+coords.ch*(coords.row+0.5))
    await sleep(300)
    await page.mouse.click(coords.x+coords.cw*10, coords.y+coords.ch*(coords.row+0.5))
    await page.keyboard.up('Control')
    hyperlinkActivated = await page.evaluate(expected => openedLinks.includes(expected), url)
  }
  const linkMark = output.length
  await press('c'); await sleep(300)
  await capture('05-copy-link')
  const linkText = `\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\`
  const singleLink = output.split(linkText).length === 2
  const fullLink = output.includes(`\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\`)
  const exactCopy = (await page.evaluate(() => copies)).some(payload => Buffer.from(payload,'base64').toString() === url)
  // A terminal OpenTUI reports without OSC 52: honest guidance, no raw request.
  const osc52Declined = options['expect-osc52'] === 'declined'
  const honest = !output.slice(copyMark).includes('selection copied to clipboard') && (osc52Declined
    ? output.slice(linkMark).includes('Drag-select the URL, then press Cmd-C') && !output.includes('\x1b]52;')
    : output.slice(linkMark).includes('Drag-select the URL, then press Cmd-C'))
  let deviceLink = false
  let transcriptLink = false
  let linkViewOnce = false
  if (fullLink) {
  await press(options['return-key'] === 'ctrl-c' ? '\x03' : '\r')
  // The same prompt regains focus after the handoff, with mouse mode restored.
  await sleep(600)
  await capture('05b-returned')
  transcriptLink = await page.evaluate(() => terminalScreen().includes('https://claude.ai/oauth/authorize?client_id=fixture'))
  // A second status check of the same waiting login must not hand off again.
  const statusMark = output.length
  await press('\x15'); await typeText('/provider login-status fixture'); await press('\r')
  await waitFor(() => requests.filter(request => request === 'GetProviderLoginStatus').length >= 2)
  await sleep(800)
  await capture('05c-status-again')
  linkViewOnce = !output.slice(statusMark).includes('Provider authorization link')
  if (!linkViewOnce) { await press('\r'); await sleep(600) }
  await press('\x15')
  for (const c of '/provider login codex') { tui.terminal.write(c); await sleep(15) }
  await press('\r')
  await sleep(700)
  await capture('06-codex-link')
  deviceLink = output.includes(`\x1b]8;;${deviceUrl}\x1b\\${deviceUrl}\x1b]8;;\x1b\\`)
  await press('\r')
  }
  result = { items: ['MP-08','MP-10','MP-11'], cli, cliSha256: await hashClient(path.dirname(cli)), kernelBinary: options['kernel-binary'] ?? null, kernelSha256: options['kernel-binary'] ? await hashFile(options['kernel-binary']) : null, source: options.source, dpr: Number(options.dpr ?? 1), mouse: !options['no-mouse'], fragmentMouse: Boolean(options['fragment-mouse']), nativeCopy, singleLink, retained, keyboardCopy, fullLink, exactCopy, nativeSelection, hyperlinkActivated, honest, deviceLink, requests, upstreamResponses, transcriptLink, linkViewOnce, themes, term: options.term ?? 'xterm-256color', expectOsc52: options['expect-osc52'] ?? 'supported', selectionColors: {before,during,after}, acceptance: 'fixture login payloads; macOS Terminal.app clipboard/Cmd-click require the coordinator desktop check' }
  await writeFile(path.join(evidence, 'terminal.pty'), output)
  console.log(JSON.stringify(result))
  const copied = osc52Declined ? !exactCopy : exactCopy
  if (options['expect-red']) assert.ok(!retained || !keyboardCopy || !fullLink || !copied || !honest || !deviceLink || !transcriptLink || !linkViewOnce, 'baseline must fail')
  else assert.ok(retained && singleLink && fullLink && copied && nativeSelection && hyperlinkActivated && honest && deviceLink && transcriptLink && linkViewOnce, 'selection/link regression')
  if (!options['expect-red'] && !options['no-mouse']) assert.ok(osc52Declined ? !keyboardCopy : keyboardCopy, 'F6 must copy the retained selection only through OSC 52 support')
  if (!options['expect-red']) assert.ok(themes.every(cell => cell.highlighted && cell.retained && cell.copied), 'theme notifications must preserve waiting-room selection and F6 copy')
  if (kernelUrl) assert.ok(upstreamResponses.some(entry => entry.request === 'ListProviderAccountProfiles' && entry.response === 'ProviderAccountProfilesListed'), 'ordinary account inventory must come from the owned real kernel')
  }
  }
} finally {
  await browser?.close()
  await stop(tui)
  if (!sessionDeleted) await deleteSession().catch(error => console.error(`session cleanup failed: ${error?.message}`))
  for (const socket of fixture.clients) socket.terminate()
  await new Promise(resolve => fixture.close(resolve))
  await Promise.allSettled([...upstreamClients].map(client => client.close()))
  frontend?.stop(true)
  await stop(kernel)
  await rm(scratch, { recursive: true, force: true })
  if (!options.interactive) await writeFile(path.join(evidence, 'terminal.pty'), output)
  await writeFile(path.join(evidence, 'result.json'), JSON.stringify({ source: options.source, cli, cliSha256: await hashClient(path.dirname(cli)), kernelBinary: options['kernel-binary'] ?? null, kernelSha256: options['kernel-binary'] ? await hashFile(options['kernel-binary']) : null, ...result, requests, upstreamResponses, cleanup: 'owned TUI/kernel/browser/proxy stopped; disposable state removed' }, null, 2))
}
