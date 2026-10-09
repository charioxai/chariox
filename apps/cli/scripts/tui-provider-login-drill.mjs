#!/usr/bin/env bun
// MP-08/MP-11: provider login interaction through the real built TUI in a PTY,
// rendered by xterm.js in Chromium, attached to an own real kernel that runs
// the official `claude setup-token`. No login is ever completed: the real flow
// stops at the code prompt, then sends a deliberately invalid code and cancels.
// --fixture-claude yes swaps in a local stand-in CLI for the success path only.
//
// bun apps/cli/scripts/tui-provider-login-drill.mjs --cli apps/cli/dist/index.js \
//   --kernel-binary <chariox-kernel> --tools <dir with playwright + @xterm/xterm> \
//   --output <evidence dir> --dpr 1|2 --profile xterm|terminal-app [--fixture-claude yes] [--expect-red yes]
// Dry run on an existing kernel (never sends a code; cancels at the code prompt):
//   --kernel-url <ws url> --chariox-home <its home> --account <claude profile label> --session-args '<TUI provider args>'
// A standalone TUI uses --compiled yes --cli <chariox> --ipc-module <built ipc.js>.
// --launcher <tonight-login.sh> drives the actual owner launcher in the PTY.
import assert from 'node:assert/strict'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'

const options = Object.fromEntries(process.argv.slice(2).reduce((pairs, item, i, args) => i % 2 ? pairs : [...pairs, [item.replace(/^--/, ''), args[i + 1]]], []))
const cli = path.resolve(options.cli ?? 'apps/cli/dist/index.js')
const compiled = options.compiled === 'yes'
const evidence = path.resolve(options.output)
const profile = options.profile ?? 'xterm'
const fixture = options['fixture-claude'] === 'yes'
const attach = Boolean(options['kernel-url'])
assert.ok(!options.launcher || attach, 'the owner launcher requires the existing-kernel dry-run mode')
assert.ok(!attach || !fixture, 'an existing kernel must never use the fixture success flow')
assert.ok(!attach || options.account === 'disposable-claude', 'existing-kernel login drills target only disposable-claude')
assert.ok(['xterm', 'terminal-app'].includes(profile), 'profile must be xterm or terminal-app')
assert.ok(!evidence.startsWith(`${process.cwd()}/`), 'evidence must be outside the repository')
assert.ok(process.env.TMPDIR && !process.env.TMPDIR.startsWith('/tmp'), 'TMPDIR must be on disk')
const { chromium } = createRequire(path.join(path.resolve(options.tools), 'package.json'))('playwright')
const sleep = ms => Bun.sleep(ms)
const sha256 = data => createHash('sha256').update(data).digest('hex')
const waitFor = async (predicate, timeoutMs = 30_000, label = 'condition') => {
  const deadline = Date.now() + timeoutMs
  while (!await predicate()) { if (Date.now() > deadline) throw Error(`timed out waiting for ${label}`); await sleep(100) }
}
// The whole built client directory, so a result names exactly what ran.
const hashTree = async root => {
  const hash = createHash('sha256')
  const visit = async (dir, prefix) => {
    for (const entry of (await readdir(dir, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      if (entry.isDirectory()) await visit(path.join(dir, entry.name), `${prefix}${entry.name}/`)
      else if (entry.isFile()) { hash.update(`${prefix}${entry.name}\0`); hash.update(await readFile(path.join(dir, entry.name))) }
    }
  }
  await visit(root, '')
  return hash.digest('hex')
}
const freePort = () => { const l = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } }); const p = l.port; l.stop(true); return p }

const scratch = await mkdtemp(path.join(process.env.TMPDIR, 'loginux-'))
const home = path.join(scratch, 'home'), state = path.join(scratch, 'state'), workspace = path.join(scratch, 'workspace')
for (const dir of [home, state, workspace, evidence]) await mkdir(dir, { recursive: true })
const env = { ...process.env }
for (const key of Object.keys(env)) if (/^(CHARIOX_|CLAUDE_|ANTHROPIC_|CODEX_|OPENCODE_|SSH_|TERM_PROGRAM)/.test(key)) delete env[key]
Object.assign(env, { HOME: home, CHARIOX_HOME: state, CHARIOX_LOG_DIR: path.join(scratch, 'logs'),
  XDG_CONFIG_HOME: path.join(scratch, 'xdg-config'), XDG_STATE_HOME: path.join(scratch, 'xdg-state'), XDG_DATA_HOME: path.join(scratch, 'xdg-data'),
  PATH: `/root/.local/bin:${process.env.PATH}`, BROWSER: '/bin/false', DISPLAY: '' })
if (fixture) {
  // Stand-in for the official CLI: same prompts, a synthetic token, a
  // successful zero-cost check. It never contacts a provider.
  const bin = path.join(scratch, 'fixture-claude.mjs')
  await writeFile(bin, `#!/usr/bin/env node
const args = process.argv.slice(2)
if (args[0] === '--version') { console.log('2.1.292 (Claude Code)'); process.exit(0) }
if (args[0] === '-p') {
  const usage = { input_tokens: 0, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 0 }
  console.log(JSON.stringify({ type: 'result', subtype: 'success', is_error: false, duration_api_ms: 0, num_turns: 0, total_cost_usd: 0, usage,
    result: 'Current session: 1% used\\nCurrent week (all models): 2% used' }))
  process.exit(0)
}
if (args[0] === 'auth') { console.log(JSON.stringify({ loggedIn: true, authMethod: 'oauth_token', apiProvider: 'firstParty' })); process.exit(0) }
if (args[0] === 'setup-token') {
  process.stdout.write('Browser didn\\'t open? Use the url below to sign in (c to copy)\\r\\n\\r\\nhttps://claude.ai/oauth/authorize?code=true&client_id=fixture-client&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&code_challenge=FixtureChallenge0123456789abcdefghijklmnopqrstu&code_challenge_method=S256&state=FixtureState0123456789abcdefghijklmnopq\\r\\n\\r\\nPaste code here if prompted > ')
  process.stdin.setRawMode?.(true)
  let code = ''
  process.stdin.on('data', chunk => {
    code += chunk.toString()
    if (!/[\\r\\n]/.test(code)) return
    process.stdout.write('\\r\\n\\r\\nLong-lived authentication token created successfully!\\r\\n\\r\\nYour OAuth token (valid for 1 year):\\r\\n\\r\\nsk-ant-oat01-' + 'F'.repeat(95) + 'AA\\r\\n\\r\\nStore this token securely.\\r\\n')
    setTimeout(() => process.exit(0), 200)
  })
} else process.exit(2)
`)
  await chmod(bin, 0o700)
  env.CHARIOX_CLAUDE_BIN = bin
}

const label = attach ? options.account : `drill-${profile}-${process.pid}`
const sessionAlias = options.launcher ? `miguel-claude-login-drill-${process.pid}` : attach ? `loginux-dry-${process.pid}` : label
const steps = []
const record = (name, ok, detail = {}) => { steps.push({ name, ok, ...detail }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`) }
let kernel, tui, browser, frontend, output = ''
const sockets = new Set()
let result
try {
  let kernelUrl = options['kernel-url']
  if (attach) {
    // The client finds the existing kernel's authorization in its home.
    env.CHARIOX_HOME = path.resolve(options['chariox-home'])
  } else {
    const kernelPort = freePort(), mcpPort = freePort()
    kernelUrl = `ws://127.0.0.1:${kernelPort}/kernel`
    kernel = Bun.spawn([path.resolve(options['kernel-binary'])], {
      env: { ...env, CHARIOX_KERNEL_PORT: String(kernelPort), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_DAEMON_SOCKET: path.join(scratch, 'k.sock') },
      stdout: 'ignore', stderr: 'ignore' })
  }
  process.env.CHARIOX_HOME = env.CHARIOX_HOME
  const { LocalIpcClient } = await import(path.resolve(options['ipc-module'] ?? path.join(path.dirname(cli), 'ipc.js')))
  const rpc = new LocalIpcClient(kernelUrl, {})
  await waitFor(async () => {
    if (kernel && kernel.exitCode !== null) throw Error(`kernel exited ${kernel.exitCode}`)
    try { return Object.keys(await rpc.send({ RelayStatus: null }))[0] === 'RelayStatus' } catch { return false }
  }, 60_000, 'kernel')
  const profileId = attach
    ? (await rpc.send({ ListProviderAccountProfiles: { provider: 'claude' } })).ProviderAccountProfilesListed.profiles
      .find(item => item.label === label)?.profile_id
    : (await rpc.send({ CreateProviderAccountProfile: { provider: 'claude', label } })).ProviderAccountProfile.profile.profile_id
  assert.ok(profileId, `Claude profile ${label}`)
  if (attach) {
    const auth = await rpc.send({ GetProviderAuthStatus: { provider: 'claude', account_profile: profileId } })
    assert.notEqual(auth.ProviderAuthStatus.status.auth_state, 'authenticated', 'never re-login an authenticated account')
  }

  frontend = Bun.serve({ hostname: '127.0.0.1', port: 0,
    fetch(request, server) {
      const route = new URL(request.url).pathname
      if (route === '/terminal' && server.upgrade(request)) return
      if (route === '/xterm.js') return new Response(Bun.file(path.join(options.tools, 'node_modules/@xterm/xterm/lib/xterm.js')))
      if (route === '/xterm.css') return new Response(Bun.file(path.join(options.tools, 'node_modules/@xterm/xterm/css/xterm.css')))
      return new Response(`<!doctype html><link rel="stylesheet" href="/xterm.css"><style>body{background:#141414;margin:16px}</style><div id="terminal"></div><script src="/xterm.js"></script><script>
        window.term=new Terminal({cols:100,rows:36,fontSize:15,fontFamily:'monospace',allowProposedApi:true,scrollback:5000});term.open(document.querySelector('#terminal'));term.focus();
        window.openedLinks=[];window.copies=[];window.ignored=[];
        ${profile === 'terminal-app'
          // Terminal.app: no OSC 8 hyperlinks, no OSC 52 clipboard, no XTVERSION reply.
          ? `term.parser.registerOscHandler(8,d=>{ignored.push('osc8');return true});term.parser.registerOscHandler(52,d=>{ignored.push('osc52');return true});term.parser.registerCsiHandler({prefix:'>',final:'q'},()=>true);`
          : `term.options.linkHandler={activate:(e,text)=>openedLinks.push(text)};term.parser.registerOscHandler(52,d=>{copies.push(d.split(';').slice(1).join(';'));return true});`}
        window.ws=new WebSocket('ws://'+location.host+'/terminal');ws.onmessage=e=>term.write(e.data);term.onData(d=>ws.send(d));
        window.screenText=()=>Array.from({length:term.rows},(_,i)=>term.buffer.active.getLine(term.buffer.active.viewportY+i)?.translateToString(true)??'').join('\\n');
        // Logical lines: rows the terminal itself soft-wrapped are joined.
        window.logicalLines=()=>{const b=term.buffer.active,out=[];for(let i=0;i<b.length;i++){const l=b.getLine(i);const t=l.translateToString(true);if(l.isWrapped&&out.length)out[out.length-1]+=t;else out.push(t)}return out};
      </script>`, { headers: { 'content-type': 'text/html' } })
    },
    websocket: { open(s) { sockets.add(s); s.send(output) }, close(s) { sockets.delete(s) }, message(_s, data) { tui?.terminal.write(String(data)) } },
  })
  browser = await chromium.launch({ headless: true, args: ['--no-sandbox'],
    ...(process.env.VALENV_CHROMIUM ? { executablePath: process.env.VALENV_CHROMIUM } : {}) })
  const page = await browser.newPage({ viewport: { width: 1000, height: 760 }, deviceScaleFactor: Number(options.dpr ?? 1) })
  await page.goto(`http://127.0.0.1:${frontend.port}`)

  const clientCommand = [...(compiled ? [cli] : ['bun', cli]), '--kernel-url', kernelUrl, '--create-session', '--alias', sessionAlias, '--workspace', workspace, '--worktree', workspace,
    // The session's own agent needs no Claude login; the login targets the new profile.
    ...(options['session-args'] ?? '--provider opencode').split(' ')]
  tui = Bun.spawn(options.launcher ? [path.resolve(options.launcher), sessionAlias] : clientCommand, {
    cwd: workspace,
    env: { ...env, TERM: 'xterm-256color', SSH_CONNECTION: '203.0.113.7 50000 198.51.100.2 22', SSH_TTY: '/dev/pts/9',
      ...(profile === 'xterm' ? { TERM_PROGRAM: 'vscode' } : {}) },
    terminal: { cols: 100, rows: 36, data(_t, chunk) { const text = new TextDecoder().decode(chunk); output += text; for (const s of sockets) s.send(text) } },
  })
  const screen = () => page.evaluate(() => screenText())
  const shows = async text => (await screen()).includes(text)
  let shot = 0
  const capture = async name => {
    await sleep(300)
    const file = `${String(++shot).padStart(2, '0')}-${name}`
    await page.screenshot({ path: path.join(evidence, `${file}.png`) })
    await writeFile(path.join(evidence, `${file}.txt`), await screen())
  }
  const press = async text => { tui.terminal.write(text); await sleep(200) }
  const typeText = async text => { for (const c of text) { tui.terminal.write(c); await sleep(12) } }
  const rows = async () => (await screen()).split('\n')
  const click = async (x, y) => { await press(`\x1b[<0;${x + 1};${y + 1}M`); await press(`\x1b[<0;${x + 1};${y + 1}m`) }

  await waitFor(async () => (await screen()).trim().length > 0 && !(await shows('Loading')), 90_000, 'TUI')
  await sleep(3000)
  await capture('attached')
  await typeText(`/provider login claude ${label}`)
  await press('\r')

  // The kernel publishes the official CLI's authorization URL on the session.
  const sessionId = (JSON.stringify(await rpc.send({ ResolveSession: { session_ref: sessionAlias, workspace_id: null } })).match(/"session_id":"([^"]+)"/) ?? [])[1]
  let loginUrl = null, loginInteraction = null
  await waitFor(async () => {
    const session = await rpc.send({ GetSessionState: { session_id: sessionId } })
    const find = value => Array.isArray(value) ? value.map(find).find(Boolean)
      : value && typeof value === 'object' ? (value.provider_login?.login?.auth_url ? value : Object.values(value).map(find).find(Boolean)) : null
    loginInteraction = find(session)
    loginUrl = loginInteraction?.provider_login.login.auth_url ?? null
    return Boolean(loginUrl)
  }, 90_000, 'kernel login interaction with authorization URL').catch(async error => { await capture('no-login-interaction'); throw error })
  await sleep(1500)
  await capture('login-strip')
  const stripRows = await rows()
  const first = stripRows.findIndex(row => row.includes('1. Open the link (click it):'))
  const second = stripRows.findIndex(row => row.includes('2. Authorize Chariox in your browser.'))
  record('numbered steps, title and no jargon', first >= 0 && second > first && stripRows.some(r => r.includes(`Sign in to Claude · ${label}`))
    && stripRows.some(r => r.includes('3. Paste the code below and press Enter.')) && !stripRows.some(r => /Send response|interaction answered/.test(r)))
  const linkRows = first >= 0 && second > first ? stripRows.slice(first + 1, second).map(r => r.replace(/^│? /, '').trimEnd()) : []
  record('link rows hold exactly the URL, wrapped only at the edge', linkRows.join('') === loginUrl
    && linkRows.slice(0, -1).every(r => r.length === linkRows[0].length), { urlLength: loginUrl.length, urlSha256: sha256(loginUrl), rows: linkRows.length })
  const countdownOf = text => text.match(/expires in (\d+):(\d\d)/)?.slice(1).map(Number)
  const countdownA = countdownOf(await screen())
  record('code field focused', stripRows.some(r => r.includes('> Code: <paste the code>▏')))
  const osc8 = output.includes(`;${loginUrl}\x1b\\`) || output.includes(`;${loginUrl}\x07`)
  // Terminal.app (no TERM_PROGRAM over SSH) gets plain text; the click/plain view covers it.
  record('OSC 8 hyperlink carries the complete URL', profile === 'terminal-app' || osc8, { osc8 })
  await sleep(2500)
  const countdownB = countdownOf(await screen())
  record('countdown runs', Boolean(countdownA && countdownB) && countdownB[0] * 60 + countdownB[1] < countdownA[0] * 60 + countdownA[1], { countdownA, countdownB })

  // C copies through the honest OSC 52 gate.
  await press('c')
  await sleep(500)
  const copied = await page.evaluate(() => copies.map(c => atob(c)))
  const footerAfterCopy = await screen()
  record('C copies the link honestly', /Link: (clipboard request sent \(OSC 52, unconfirmed\)|clipboard unavailable)/.test(footerAfterCopy) && !/copied to local clipboard/.test(footerAfterCopy)
    && (profile === 'terminal-app' || copied.includes(loginUrl)), { osc52Copies: copied.length, ignored: await page.evaluate(() => ignored) })
  await capture('after-copy')

  // Mouse reporting stays on; a click on the link (SSH) shows the plain view.
  const linkRow = (await rows()).findIndex(r => r.includes(loginUrl.slice(0, 40)))
  if (linkRow >= 0) await click(20, linkRow)
  await sleep(800)
  await capture('click-plain-view')
  const logical = await page.evaluate(() => logicalLines())
  record('click shows the link as one terminal-wrapped logical line', logical.some(l => l.trimEnd() === loginUrl) && await shows('1. Open this link (Cmd-click it, or select and copy it):'))

  if (attach) {
    // MP-08/MP-11: shared kernel dry run. Fill only the TUI's masked field
    // with synthetic text, splitting both delimiters as SSH may do. Never
    // submit it to the official provider or complete a real authorization.
    if (options['split-paste'] === 'yes') {
      const code = 'dry#state'
      for (const chunk of ['\x1b', '[20', '0~' + code + '\x1b[20', '1~']) await press(chunk)
      await sleep(800)
      await capture('split-paste-masked')
      record('split paste returns with the masked code without submitting it',
        (await rows()).some(r => r.includes(`Code: ${'*'.repeat(code.length)}▏`)) && !(await shows(code)))
      // Also lets the before-fix client leave a plain view after losing paste.
      if (await shows('1. Open this link')) await press('\r')
    } else await press('\r')
    await sleep(800)
    await capture('returned')
    record('returns to the focused code field', (await rows()).some(r => r.includes(options['split-paste'] === 'yes' ? `> Code: ${'*'.repeat('dry#state'.length)}▏` : '> Code: <paste the code>▏')))
  } else {
    // The code pasted in the plain view returns to Chariox and fills the field.
    // Without a plain view (the client before this change) the user selects
    // "2.Send response" first, as Miguel did.
    const code = 'invalidDrillCode0123456789#invalidDrillState'
    if (!await shows('1. Open this link')) await press('2')
    await press(`\x1b[200~${code}\x1b[201~`)
    await sleep(1200)
    await capture('code-pasted')
    record('paste returns with the masked code', (await rows()).some(r => r.includes(`Code: ${'*'.repeat(24)}▏`)) && !(await shows(code)))
    await press('\r')
    await sleep(400)
    await capture('code-sent')
    // A wrong code may be answered before the next frame; the PTY keeps the progress text.
    record('progress, never only "interaction answered"', /checking the code|Checking the code/.test(output) && !output.includes('interaction answered'))

    if (fixture) {
      // Disposable kernel: Chariox Vault is created through its own prompts.
      await waitFor(() => shows('Create Chariox Vault'), 60_000, 'vault prompt')
      await capture('vault-create')
      for (const prompt of ['Create Chariox Vault', 'Confirm Chariox Vault']) {
        await waitFor(() => shows(prompt), 30_000, prompt)
        // Choose the passphrase field unless it already has the cursor.
        if (!(await rows()).some(r => /> 2\.(Confirm vault passphrase|Vault passphrase): .*_\s*$/.test(r))) await press('2')
        await typeText('drill-vault-passphrase'); await capture(`vault-${prompt.split(' ')[0].toLowerCase()}`); await press('\r'); await sleep(1500)
      }
      await waitFor(() => shows('Signed in to Claude'), 60_000, 'sign-in result').catch(async error => { await capture('no-result'); throw error })
      await capture('signed-in')
      const auth = await rpc.send({ GetProviderAuthStatus: { provider: 'claude', account_profile: profileId } })
      record('result names where the account was saved', (await screen()).includes(`Signed in to Claude`) && (await screen()).includes(`saved to ${label}`)
        && auth.ProviderAuthStatus.status.auth_state === 'authenticated', { authState: auth.ProviderAuthStatus.status.auth_state })
    } else {
      // Official CLI, invalid code: the provider's own error is shown.
      await waitFor(async () => /OAuth error|error|invalid/i.test((await rows()).slice(0, 34).join('\n')) || await shows('sign-in failed'), 45_000, 'provider verdict').catch(() => {})
      await capture('code-rejected')
      const rejected = /OAuth error: Request failed with status code 4\d\d/.test(await screen()) || await shows('sign-in failed')
      // The provider drops its link after a rejection: the strip says what to do next.
      const nextStep = await shows('This link expired. Press Esc, then 1 to cancel, and sign in again.') || await shows('1. Open the link (click it):')
      record('the provider rejection is visible with the next step', rejected && nextStep, { rejected, nextStep })
      // Esc leaves the field; 1 cancels through the kernel interaction.
      await press('\x1b'); await sleep(300); await press('1')
      await waitFor(() => shows('sign-in cancelled'), 30_000, 'cancel result').catch(() => {})
      await capture('cancelled')
      record('cancel ends with a result and a retry action', await shows(`Claude · ${label}: sign-in cancelled`) && await shows(`Retry: /provider login claude ${label}`))
    }
  }
  if (attach) {
    await press('\x1b'); await sleep(300); await press('1')
    await waitFor(() => shows('sign-in cancelled'), 30_000, 'cancel result').catch(() => {})
    await capture('cancelled')
    record('cancel ends with a result and a retry action', await shows(`Claude · ${label}: sign-in cancelled`) && await shows(`Retry: /provider login claude ${label}`))
  }
  {
    const auth = await rpc.send({ GetProviderAuthStatus: { provider: 'claude', account_profile: profileId } })
    record(fixture ? 'only the synthetic fixture was authenticated' : 'no login was completed',
      fixture ? auth.ProviderAuthStatus.status.auth_state === 'authenticated' : auth.ProviderAuthStatus.status.auth_state !== 'authenticated',
      { authState: auth.ProviderAuthStatus.status.auth_state })
  }
  if (options.launcher) {
    await press('\x05')
    await waitFor(() => tui.exitCode !== null, 30_000, 'launcher exit and session cleanup')
    record('owner launcher exits cleanly', tui.exitCode === 0)
  } else {
    await rpc.send({ DeleteSession: { session_ref: sessionAlias, workspace_id: null } }).catch(() => {})
  }
  await rpc.close()
  await writeFile(path.join(evidence, 'pty-output.log'), output.replaceAll(loginUrl, '<authorization URL>'))
  result = { items: ['MP-08', 'MP-11'], profile, dpr: Number(options.dpr ?? 1), fixtureClaude: fixture, cli,
    ...(compiled ? { cliSha256: sha256(await readFile(cli)) } : { cliDistSha256: await hashTree(path.dirname(cli)) }),
    kernel: attach ? kernelUrl : { sha256: sha256(await readFile(path.resolve(options['kernel-binary']))) }, steps }
} finally {
  const stop = async child => {
    if (!child || child.exitCode !== null) return
    assert.ok(Number.isInteger(child.pid) && child.pid > 1, 'reject unsafe PID')
    child.kill('SIGTERM')
    await Promise.race([child.exited, sleep(4000)])
    if (child.exitCode === null) child.kill('SIGKILL')
    await child.exited
  }
  await stop(tui)
  await browser?.close()
  frontend?.stop(true)
  await stop(kernel)
  await rm(scratch, { recursive: true, force: true })
}
await writeFile(path.join(evidence, 'result.json'), JSON.stringify(result, null, 2))
const green = result.steps.every(step => step.ok)
console.log(JSON.stringify({ green, failed: result.steps.filter(s => !s.ok).map(s => s.name) }))
if (options['expect-red']) assert.ok(!green, 'base client must fail the login interaction checks')
else assert.ok(green, 'login interaction checks')
