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
const clientDir = path.resolve(options['client-dir'] ?? path.dirname(cli))
const nativeSelection = options['native-selection'] || (!options.attached && !options['waiting-room-paste'])
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
  ...(options.attached || options['waiting-room-paste'] ? { CHARIOX_TUI_MOUSE: 'on' } : {}),
})
await mkdir(runtimeEnv.HOME, { recursive: true })
await mkdir(evidence, { recursive: true })
const url = options['link-url'] ?? (nativeSelection ? 'https://github.com/charioxai/chariox' : 'https://claude.ai/oauth/authorize?client_id=fixture&redirect_uri=https%3A%2F%2Fexample.org%2Fcallback&state=' + '0123456789abcdef'.repeat(25))
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
let sessionDeleted = !options.attached && !options['native-session']
const deleteSession = async () => {
  const client = new KernelClient(options['native-session'] ? kernelUrl : options['fleet-kernel-url'], {})
  try { await client.send({ DeleteSession: { session_ref: sessionAlias, workspace_id: null } }); sessionDeleted = true }
  finally { await client.close() }
}
// Kernel-side status of the drill's queued prompt (authoritative "not cancelled").
const queuedPromptStatus = async prompt => {
  const client = new KernelClient(options['native-session'] ? kernelUrl : options['fleet-kernel-url'], {})
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
    KernelClient = (await import(path.join(clientDir, 'ipc.js'))).LocalIpcClient
    kernel = Bun.spawn([path.resolve(options['kernel-binary'])], { env: { ...runtimeEnv, CHARIOX_HOME: path.join(scratch, 'state'), CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_DAEMON_SOCKET: path.join(scratch,'kernel.sock'), CHARIOX_LOG_DIR: path.join(scratch, 'logs') }, stdout: 'ignore', stderr: 'ignore' })
    const probe = new KernelClient(kernelUrl, {})
    upstreamClients.add(probe)
    // The kernel serves WebSocket RPCs, not an HTTP /health endpoint.
    await waitFor(async () => {
      if (kernel.exitCode !== null) throw Error(`owned kernel exited: ${kernel.exitCode}`)
      try { return Object.keys(await probe.send({ RelayStatus: null }))[0] === 'RelayStatus' } catch { return false }
    })
    if (options['native-session']) {
      const {linkProviderAccountProfile,refreshProviderAccountProfile} = await import(path.join(clientDir,'provider-api.js'))
      const profile = await linkProviderAccountProfile(probe,'codex','loginux-native',options['provider-scope'])
      const refreshed = await refreshProviderAccountProfile(probe,'codex',profile.profile_id)
      assert.equal(refreshed.auth_state,'authenticated','product-linked provider must be authenticated')
      options['account-profile'] = profile.profile_id
    }
    await probe.close()
  }
  if (options.attached) {
    // Normal product client authorization from the account-holding kernel home.
    process.env.CHARIOX_HOME = path.resolve(options['fleet-home'])
    KernelClient = (await import(path.join(clientDir, 'ipc.js'))).LocalIpcClient
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
    tui = Bun.spawn([...(cli.endsWith('.js') ? ['bun', cli] : [cli]), '--detached', '--kernel-url', `ws://127.0.0.1:${fixture.address().port}/kernel`], {
      cwd: process.cwd(), env: runtimeEnv,
      stdin: 'inherit', stdout: 'inherit', stderr: 'inherit',
    })
    await tui.exited
    result = { items: ['MP-08','MP-11'], source: options.source, cli, cliSha256: await (cli.endsWith('.js') ? hashClient(path.dirname(cli)) : hashFile(cli)), requests, interactive: true, acceptance: 'manual macOS/SSH observations must be recorded by the operator' }
  } else {
  frontend = Bun.serve({ hostname: '127.0.0.1', port: 0,
    fetch(request, server) {
      const route = new URL(request.url).pathname
      if (route === '/terminal' && server.upgrade(request)) return
      if (route === '/xterm.js') return new Response(Bun.file(path.join(tools, 'node_modules/@xterm/xterm/lib/xterm.js')))
      if (route === '/xterm.css') return new Response(Bun.file(path.join(tools, 'node_modules/@xterm/xterm/css/xterm.css')))
      return new Response(`<!doctype html><link rel="stylesheet" href="/xterm.css"><style>body{background:#141414;margin:16px}</style><button id="native-copy" onmousedown="event.preventDefault()" onclick="document.execCommand('copy')">Terminal Copy</button><div id="terminal"></div><script src="/xterm.js"></script><script>
        window.term=new Terminal({cols:100,rows:35,fontSize:16,fontFamily:'monospace',allowProposedApi:true,scrollback:10000});term.open(document.querySelector('#terminal'));term.focus();
        window.openedLinks=[];term.options.linkHandler={activate:(event,text)=>{openedLinks.push(text);${nativeSelection ? "window.openedWindow=Boolean(window.open(text,'_blank'))" : ''}}};
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
  const tuiArgs = options['native-session']
    ? ['--kernel-url', kernelUrl, '--create-session', '--alias', sessionAlias, '--workspace', process.cwd(), '--worktree', process.cwd(), '--provider', 'codex', '--account-profile', options['account-profile'], '--model', 'codex/gpt-6.1-sol']
    : options['waiting-room-paste']
    ? ['--kernel-url', kernelUrl]
    : options.attached
    ? ['--kernel-url', options['fleet-kernel-url'], '--create-session', '--alias', sessionAlias, '--workspace', process.cwd(), '--worktree', process.cwd(),
      '--provider', options.provider ?? 'codex', '--account-profile', options['account-profile'], '--model', options.model]
    : ['--detached', '--kernel-url', `ws://127.0.0.1:${fixture.address().port}/kernel`]
  tui = Bun.spawn([...(cli.endsWith('.js') ? ['bun', cli] : [cli]), ...tuiArgs], {
    cwd: process.cwd(), env: { ...runtimeEnv, TERM: options.term ?? 'xterm-256color', SSH_CONNECTION: 'fixture 1 fixture 2',
      ...(options['no-mouse'] ? { CHARIOX_TUI_MOUSE: 'off' } : {}),
      ...(options.attached || options['waiting-room-paste'] || options['native-session'] ? { CHARIOX_HOME: process.env.CHARIOX_HOME } : {}) },
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
    const rows = terminalScreen().split('\n'); const y = rows.findLastIndex(row => row.includes(n))
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
  const promptShows = text => page.evaluate(t => terminalScreen().split('\n').slice(-8).some(row => row.includes(t)), text)
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
  if (nativeSelection) {
    // MP-08/MP-10/MP-11: browser mouse events, real PTY bytes, and the
    // terminal's own Copy action. No F-key, Shift-drag, OSC 52 or select API.
    let selectionText = 'Provider Accounts'
    if (options['native-session']) {
      await waitFor(() => page.evaluate(() => terminalScreen().includes('Write your next prompt here')),90_000)
      await typeText('Read the public README at https://github.com/charioxai/chariox, then return NATIVE COPY READY on one line and that repository URL alone on the next line. ' + (options['code-fence'] ? 'Put only those two lines in a single fenced text code block. No explanation.' : 'No explanation.'))
      await press('\r')
      await waitFor(() => page.evaluate(() => terminalScreen().split('\n').some(row => row.replace(/^[│ ]+/,'').replace(/[█ ]+$/,'') === 'NATIVE COPY READY') && /\bIDLE\b/.test(terminalScreen())),180_000)
      selectionText = 'NATIVE COPY READY'
    } else await waitFor(() => page.evaluate(() => terminalScreen().includes('Provider Accounts')))
    await capture('native-01-open')
    const at = await settledRowOf(selectionText)
    const coords = await page.evaluate(({x,y}) => {
      const rect = document.querySelector('.xterm-screen').getBoundingClientRect()
      return { x: rect.x+(x+0.2)*rect.width/term.cols, y: rect.y+(y+0.5)*rect.height/term.rows, cw: rect.width/term.cols }
    }, at)
    await page.mouse.move(coords.x, coords.y); await page.mouse.down()
    await page.mouse.move(coords.x+selectionText.length*coords.cw, coords.y, {steps:20}); await page.mouse.up()
    await sleep(400)
    const selected = await page.evaluate(() => term.getSelection())
    await capture('native-02-drag')
    const mouseEnabled = /\x1b\[\?(?:1000|1002|1003|1006)h/.test(output)
    result = {items:['MP-08','MP-10','MP-11'],source:options.source,mouseEnabled,selected}
    assert.ok(!mouseEnabled && selected === selectionText, 'plain drag must create native terminal selection without F7')
    await page.context().grantPermissions(['clipboard-read', 'clipboard-write'])
    await page.click('#native-copy')
    const clipboard = await page.evaluate(() => navigator.clipboard.readText()).catch(() => null)
    await capture('native-03-copy')
    let multilineSelected = null
    if (options['native-session']) {
      const link = await rowOf(url)
      const end = await page.evaluate(({x,y,length}) => {
        const rect=document.querySelector('.xterm-screen').getBoundingClientRect()
        return {x:rect.x+(x+length+0.2)*rect.width/term.cols,y:rect.y+(y+0.5)*rect.height/term.rows}
      }, {...link,length:url.length})
      await page.mouse.move(coords.x,coords.y); await page.mouse.down()
      await page.mouse.move(end.x,end.y,{steps:20}); await page.mouse.up()
      multilineSelected = await page.evaluate(() => term.getSelection())
      await page.click('#native-copy')
      const multilineClipboard = await page.evaluate(() => navigator.clipboard.readText())
      await capture('native-03b-multiline')
      result = {...result,multilineSelected,multilineClipboard}
      assert.equal(multilineSelected,selectionText+'\n'+url,'native multiline copy excludes gutters and padding')
      assert.equal(multilineClipboard,multilineSelected)
    }
    if (!options['native-session']) {
    // Keyboard navigation is the same public user flow in native mouse mode.
    for (let i=0;i<65;i++) {
      if (await page.evaluate(() => />\s+Provider Accounts/.test(terminalScreen()))) break
      await press('\x1b[A')
    }
    await press('\r')
    await waitFor(() => page.evaluate(() => terminalScreen().includes('/provider accounts')))
    await press('\x15'); await typeText('/provider login-status fixture'); await press('\r')
    await waitFor(() => requests.includes('GetProviderLoginStatus'))
    await sleep(800)
    await capture('native-04-link')
    }
    // MP-08/MP-10/MP-11: real terminal resize forces the existing linked
    // buffers to repaint, exposing expired native URL metadata.
    await page.evaluate(() => term.resize(101,35)); tui.terminal.resize(101,35)
    await sleep(400)
    await page.evaluate(() => term.resize(100,35)); tui.terminal.resize(100,35)
    await sleep(500)
    await capture('native-04b-repaint')
    const mark = output.length
    const linkAt = await rowOf(url) ?? await rowOf(url.slice(0,32))
    // A normal click on a blank terminal cell clears the previous native
    // selection before following a link; no selection API or TUI mouse event.
    const blank = await page.evaluate(() => {
      const rect=document.querySelector('.xterm-screen').getBoundingClientRect()
      return {x:rect.right-5,y:rect.bottom-5}
    })
    await page.mouse.click(blank.x,blank.y)
    await sleep(200)
    let activated = false
    const popupPromise = page.waitForEvent('popup',{timeout:30_000}).catch(() => null)
    if (linkAt) {
      const linkCoords = await page.evaluate(({x,y}) => {
        const rect = document.querySelector('.xterm-screen').getBoundingClientRect()
        return {x:rect.x+(x+8)*rect.width/term.cols,y:rect.y+(y+0.5)*rect.height/term.rows}
      }, linkAt)
      await page.keyboard.down('Control')
      await page.mouse.move(linkCoords.x,linkCoords.y); await sleep(400)
      await page.mouse.click(linkCoords.x,linkCoords.y)
      await page.keyboard.up('Control'); await sleep(500)
      activated = await page.evaluate(expected => openedLinks.includes(expected),url)
    }
    const popup = await popupPromise
    const popupDiagnostic = await page.evaluate(() => ({requested:window.openedWindow,links:openedLinks}))
    const openedPage = popup ?? page.context().pages().find(value => value !== page)
    let browserUrl = openedPage ? openedPage.url() : null
    let browserTitle = null
    if (openedPage) {
      await openedPage.waitForURL(value => value.href.startsWith(url),{timeout:30_000}).catch(() => {})
      browserUrl = openedPage.url()
      await openedPage.waitForLoadState('domcontentloaded',{timeout:15_000}).catch(() => {})
      browserTitle = await openedPage.title().catch(() => null)
      await openedPage.screenshot({path:path.join(evidence,'native-05b-browser.png')}).catch(() => {})
    }
    await capture('native-05-click')
    result = {items:['MP-08','MP-10','MP-11'],mode:'native-selection',source:options.source,
      cli,multilineSelected,cliSha256:await (cli.endsWith('.js') ? hashClient(path.dirname(cli)) : hashFile(cli)),mouseEnabled,selected,clipboard,
      browserUrl,browserTitle,popupDiagnostic,resizedBeforeClick:true,realProvider: Boolean(options['native-session']), kernelSource:options['kernel-source'] ?? null,kernelSha256: options['kernel-binary'] ? await hashFile(options['kernel-binary']) : null,
      osc8:[...output.matchAll(/\x1b\]8;[^;\x1b\x07]*;([^\x1b\x07]*)/g)].some(match => match[1] === url),activated,
      checkEndByte:output.length,leftScreen:/\x1b\[\?1049l/.test(output),clickLeftScreen:/\x1b\[\?1049l/.test(output.slice(mark)),
      acceptance:options['native-session'] ? 'Real standalone TUI, owned kernel, product-linked official Codex turn, public repository and native terminal clipboard/browser action; local transport. Physical Mac and hosted WSS acceptance remain separate.' : 'Real built CLI/PTY/browser terminal regression with fixture login response. Native macOS Cmd-C/Cmd-click requires a physical desktop; Linux Ctrl-click is not macOS acceptance.'}
    console.log(JSON.stringify(result))
    await writeFile(path.join(evidence,'result.json'),JSON.stringify(result,null,2))
    assert.ok(!mouseEnabled && selected === selectionText && clipboard === selected && result.osc8 && activated && browserUrl?.startsWith(url) && !result.leftScreen,'native selection/copy/link must stay in TUI')
  } else if (options['waiting-room-paste']) {
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
      cli, cliSha256: await (cli.endsWith('.js') ? hashClient(path.dirname(cli)) : hashFile(cli)), kernelSha256: await hashFile(options['kernel-binary']),
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
    await waitFor(async () => (await rowOf(marker)) !== null, 240_000)
    await sleep(4000)
    await capture('a02-response')
    if (options['selection-review']) {
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
      const pasted = options['paste-review'] ? await pasteCases(marker) : []
      await typeText('zq'); await sleep(500)
      const typedAfterDrag = await promptShows('zq')
      const clearedByTyping = JSON.stringify(await cellColors(at, 13)) === JSON.stringify(before)
      await press('\x15'); await press(copySequence)
      const emptyCopyKeptAlive = tui.exitCode === null
      await capture('a05-typed-and-empty-copy')
      result = { items: ['MP-08', 'MP-10'], mode: 'selection-review', source: options.source,
        cli, cliSha256: await (cli.endsWith('.js') ? hashClient(path.dirname(cli)) : hashFile(cli)), kernelUrl: options['fleet-kernel-url'],
        provider: options.provider ?? 'codex', accountProfile: options['account-profile'], model: options.model,
        dpr: Number(options.dpr ?? 1), batchMouse: Boolean(options['batch-mouse']), fragmentMouse: Boolean(options['fragment-mouse']), copyKey: options['copy-key'] ?? 'f6',
        highlighted, keyboardCopy, typedAfterDrag, clearedByTyping, emptyCopyKeptAlive, pasted,
        acceptance: 'real provider and built TUI via PTY; desktop Terminal.app and hosted transport need separate observations' }
      const green = highlighted && keyboardCopy && typedAfterDrag && clearedByTyping && emptyCopyKeptAlive
        && pasted.every(cell => cell.cleared && cell.inserted)
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
    result = { items: ['MP-08','MP-11'], mode: 'attached', cli, cliSha256: await (cli.endsWith('.js') ? hashClient(path.dirname(cli)) : hashFile(cli)), source: options.source, dpr: Number(options.dpr ?? 1),
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
  if (kernel && kernel.exitCode === null) {
    const audit = Bun.spawn(['python3','-c',`
import os,json
root=int(__import__('sys').argv[1]); assert root>1
rows={}
for name in os.listdir('/proc'):
 if name.isdigit():
  try:
   v=open('/proc/'+name+'/stat').read().rsplit(')',1)[1].split(); rows[int(name)]=(int(v[1]),v[19])
  except (OSError,IndexError,ValueError): pass
owned={root}; changed=True
while changed:
 changed=False
 for pid,(parent,start) in rows.items():
  if parent in owned and pid not in owned: owned.add(pid); changed=True
print(json.dumps([[pid,rows[pid][1]] for pid in owned if pid!=root and pid>1]))
`,String(kernel.pid)],{stdout:'pipe'})
    const owned = JSON.parse(await new Response(audit.stdout).text()); await audit.exited
    await stop(kernel)
    const cleanup = Bun.spawn(['python3','-c',`
import os,json,signal,sys,time
rows=json.loads(sys.argv[1]); stopped=[]; live=[]
for pid,start in rows:
 assert isinstance(pid,int) and pid>1
 try:
  fd=os.pidfd_open(pid)
  v=open('/proc/'+str(pid)+'/stat').read().rsplit(')',1)[1].split()
  if v[19]!=start or v[0]=='Z': os.close(fd); continue
  signal.pidfd_send_signal(fd,signal.SIGTERM); time.sleep(.1)
  try:
   v=open('/proc/'+str(pid)+'/stat').read().rsplit(')',1)[1].split()
   if v[19]==start and v[0]!='Z': signal.pidfd_send_signal(fd,signal.SIGKILL)
  except OSError: pass
  os.close(fd); stopped.append(pid)
 except ProcessLookupError: pass
 except FileNotFoundError: pass
print(json.dumps({'stopped':stopped}))
`,JSON.stringify(owned)],{stdout:'pipe'})
    await writeFile(path.join(evidence,'descendant-cleanup.json'),await new Response(cleanup.stdout).text()); await cleanup.exited
  } else await stop(kernel)
  await rm(scratch, { recursive: true, force: true })
  if (!options.interactive) await writeFile(path.join(evidence, 'terminal.pty'), output)
  await writeFile(path.join(evidence, 'result.json'), JSON.stringify({ source: options.source, ...result, requests, upstreamResponses, cleanup: 'owned TUI/kernel/browser/proxy stopped; disposable state removed' }, null, 2))
}
