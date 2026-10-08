#!/usr/bin/env bun
// MP-08/MP-11: real built TUI + PTY + xterm terminal. Login payloads ONLY are
// fixtures: never run real provider login/logout or touch a shared account.
import assert from 'node:assert/strict'
import path from 'node:path'
import { createHash } from 'node:crypto'
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
const options = Object.fromEntries(process.argv.slice(2).reduce((pairs, item, i, args) => i % 2 ? pairs : [...pairs, [item.replace(/^--/, ''), args[i + 1]]], []))
const cli = path.resolve(options.cli ?? 'apps/cli/dist/index.js')
const evidence = path.resolve(options.output)
const tools = options.tools ? path.resolve(options.tools) : null
assert.ok(!evidence.startsWith(`${process.cwd()}/`), 'evidence must be outside the repository')
assert.ok(process.env.TMPDIR && !process.env.TMPDIR.startsWith('/tmp'), 'TMPDIR must be on disk')
const chromium = options.interactive ? null : createRequire(path.join(tools, 'package.json'))('playwright').chromium
const scratch = await mkdtemp(path.join(process.env.TMPDIR, 'tuifix-'))
await mkdir(evidence, { recursive: true })
const url = 'https://claude.ai/oauth/authorize?client_id=fixture&redirect_uri=https%3A%2F%2Fexample.org%2Fcallback&state=' + '0123456789abcdef'.repeat(25)
const deviceUrl = 'https://auth.openai.com/codex/device'
const requests = []
let output = '', tui, browser, page, frontend, kernel
const clients = new Set()
const fixture = new WebSocketServer({ port: 0, host: '127.0.0.1' })
await new Promise(resolve => fixture.once('listening', resolve))
let kernelUrl
let KernelClient
const upstreamClients = new Set()
let result
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
const waitFor = async predicate => {
  const deadline = Date.now() + 20_000
  while (!await predicate()) { if (Date.now() > deadline) throw Error('terminal condition timed out'); await sleep(50) }
}
try {
  if (options['kernel-binary']) {
    // Own exact-source kernel. The proxy replaces login responses and forwards
    // ordinary inventory/control traffic without logging headers or payloads.
    const listener = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } })
    const port = listener.port; listener.stop(true)
    kernelUrl = `ws://127.0.0.1:${port}/kernel`
    process.env.CHARIOX_HOME = path.join(scratch, 'state')
    KernelClient = (await import(path.join(path.dirname(cli), 'ipc.js'))).LocalIpcClient
    kernel = Bun.spawn([path.resolve(options['kernel-binary'])], { env: { ...process.env, CHARIOX_HOME: path.join(scratch, 'state'), CHARIOX_KERNEL_PORT: String(port), CHARIOX_LOG_DIR: path.join(scratch, 'logs') }, stdout: 'ignore', stderr: 'ignore' })
    await waitFor(async () => { try { const res = await fetch(`http://127.0.0.1:${port}/health`); return res.status < 500 } catch { return false } })
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
        const request = f.type === 'subscribe' ? { type: 'subscribe', session_id: f.session_id, attachment_id: f.attachment_id, subscription_scope: f.subscription_scope } : req
        void upstream.send(request).then(response => {
          if (socket.readyState === 1) socket.send(JSON.stringify({ type: 'response', request_id: f.request_id, response, error: null }))
        }).catch(() => {
          if (socket.readyState === 1) socket.send(JSON.stringify({ type: 'response', request_id: f.request_id, response: { Error: { message: 'owned kernel request unavailable' } }, error: null }))
        })
      }
      else socket.send(JSON.stringify({ type: 'response', request_id: f.request_id, response: { Error: { message: 'unsupported fixture request' } }, error: null }))
    })
  })
  if (options.interactive) {
    tui = Bun.spawn(['bun', cli, '--detached', '--kernel-url', `ws://127.0.0.1:${fixture.address().port}/kernel`], {
      cwd: process.cwd(), env: { ...process.env, CHARIOX_HOME: path.join(scratch,'state'), CHARIOX_LOG_DIR: path.join(scratch,'logs') },
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
  tui = Bun.spawn(['bun', cli, '--detached', '--kernel-url', `ws://127.0.0.1:${fixture.address().port}/kernel`], {
    cwd: process.cwd(), env: { ...process.env, CHARIOX_HOME: path.join(scratch, 'state'), CHARIOX_LOG_DIR: path.join(scratch, 'logs'), TERM: 'xterm-256color', SSH_CONNECTION: 'fixture 1 fixture 2', ...(options['no-mouse'] ? { CHARIOX_TUI_MOUSE: 'off' } : {}) },
    terminal: { cols: 100, rows: 35, data(_terminal, chunk) { const data = new TextDecoder().decode(chunk); output += data; for (const c of clients) c.send(data) } },
  })
  browser = await chromium.launch({ headless: true, args: ['--no-sandbox'], executablePath: options.chromium })
  page = await browser.newPage({ viewport: { width: 1050, height: 740 }, deviceScaleFactor: Number(options.dpr ?? 1) })
  await page.goto(`http://127.0.0.1:${frontend.port}`)
  await waitFor(() => page.evaluate(() => terminalScreen().includes('Provider Accounts')))
  const capture = async name => {
    await sleep(250)
    await page.screenshot({ path: path.join(evidence, `${name}.png`) })
    await writeFile(path.join(evidence, `${name}.txt`), await page.evaluate(() => terminalScreen()))
  }
  const press = async sequence => { tui.terminal.write(sequence); await sleep(180) }
  const command = async text => {
    // The ordinary waiting-room account control stages and focuses a command.
    await press('\x1b[A'); await press('\x1b[A'); await press('\r'); await press('\x15')
    for (const c of text) { tui.terminal.write(c); await sleep(15) }
    await press('\r')
  }
  await capture('01-waiting-room')
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
  await press(`\x1b[<0;${selection.x+1};${selection.y+1}M`)
  await press(`\x1b[<32;${selection.x+16};${selection.y+1}M`)
  const during = await colors()
  await capture('02-dragging')
  const copyMark = output.length
  await press(`\x1b[<0;${selection.x+16};${selection.y+1}m`)
  await sleep(500)
  const after = await colors()
  await capture('03-released')
  const retained = JSON.stringify(before) !== JSON.stringify(during) && JSON.stringify(during) === JSON.stringify(after)
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
  const fullLink = output.includes(`\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\`)
  const exactCopy = (await page.evaluate(() => copies)).some(payload => Buffer.from(payload,'base64').toString() === url)
  const honest = !output.slice(copyMark).includes('selection copied to clipboard') && output.slice(linkMark).includes('unconfirmed')
  let deviceLink = false
  if (fullLink) {
  await press('\r')
  // The same prompt regains focus after the handoff, with mouse mode restored.
  await sleep(600)
  await press('\x15')
  for (const c of '/provider login codex') { tui.terminal.write(c); await sleep(15) }
  await press('\r')
  await sleep(700)
  await capture('06-codex-link')
  deviceLink = output.includes(`\x1b]8;;${deviceUrl}\x1b\\${deviceUrl}\x1b]8;;\x1b\\`)
  await press('\r')
  }
  result = { items: ['MP-08','MP-11'], cli, cliSha256: await hashClient(path.dirname(cli)), kernelBinary: options['kernel-binary'] ?? null, kernelSha256: options['kernel-binary'] ? createHash('sha256').update(await readFile(options['kernel-binary'])).digest('hex') : null, source: options.source, dpr: Number(options.dpr ?? 1), retained, fullLink, exactCopy, nativeSelection, hyperlinkActivated, honest, deviceLink, requests, selectionColors: {before,during,after}, acceptance: 'fixture login payloads; macOS Terminal.app clipboard/Cmd-click require the coordinator desktop check' }
  await writeFile(path.join(evidence, 'terminal.pty'), output)
  console.log(JSON.stringify(result))
  if (options['expect-red']) assert.ok(!retained || !fullLink || !exactCopy || !honest || !deviceLink, 'baseline must fail')
  else assert.ok(retained && fullLink && exactCopy && nativeSelection && hyperlinkActivated && honest && deviceLink, 'selection/link regression')
  }
} finally {
  await browser?.close()
  await stop(tui)
  for (const socket of fixture.clients) socket.terminate()
  await new Promise(resolve => fixture.close(resolve))
  await Promise.allSettled([...upstreamClients].map(client => client.close()))
  frontend?.stop(true)
  await stop(kernel)
  await rm(scratch, { recursive: true, force: true })
  if (!options.interactive) await writeFile(path.join(evidence, 'terminal.pty'), output)
  await writeFile(path.join(evidence, 'result.json'), JSON.stringify({ ...result, cleanup: 'owned TUI/kernel/browser/proxy stopped; disposable state removed' }, null, 2))
}
