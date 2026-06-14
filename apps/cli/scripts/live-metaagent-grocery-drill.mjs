#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 90_000
const DEFAULT_POLL_MS = 250

function parseArgs(argv) {
  const options = {
    keepArtifactsOnFailure: false,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-grocery-drill.mjs [options]',
        '',
        'Runs a deterministic local grocery delegation drill:',
        '- creates a fresh git repo and metaagent session',
        '- verifies metaagent plan mode and meta-only runtime tool exposure',
        '- drives metaagent delegation commands for workers and workflow wiring',
        '- materializes a local grocery web app as worker output',
        '- starts the app and runs registration/login/catalog/cart/checkout smoke checks',
        '',
        'Options:',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 58500 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
    appPort: kernelPort + 3000,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-grocery-drill] ${name}`)
  else console.log(`[metaagent-grocery-drill] ${name}`, JSON.stringify(details))
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-grocery-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Grocery Drill'], { cwd: root })
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const existing = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (existing) return binary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return binary
}

async function waitForDaemon(shellBin, kernelUrl, workspace, env) {
  const scriptPath = path.join(workspace, 'wait.arroba')
  await writeFile(scriptPath, 'session list\n', 'utf8')
  const deadline = Date.now() + 20_000
  let last = null
  while (Date.now() < deadline) {
    last = await run(process.execPath, [shellBin, 'run', scriptPath, '--kernel-url', kernelUrl, '--workspace', workspace, '--worktree', workspace], { env })
    if (last.code === 0) return
    await sleep(250)
  }
  throw new Error(`daemon did not become ready\nstdout:\n${last?.stdout ?? ''}\nstderr:\n${last?.stderr ?? ''}`)
}

function requireOutput(output, pattern, label) {
  if (!pattern.test(output)) {
    throw new Error(`missing ${label}: ${pattern}\n--- output ---\n${output}`)
  }
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function launchRuntime(client, requests, sessionId, agentId, model, timeoutMs, pollMs) {
  const launched = unwrapVariant(
    await client.send(requests.launchProviderRunRequest(sessionId, 'dev-stub', 'default', model, 'low', agentId)),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  const providerRun = launched.provider_run
  if (!providerRun?.id) throw new Error(`launch did not return provider run: ${JSON.stringify(launched)}`)
  const deadline = Date.now() + timeoutMs
  let last = providerRun
  while (Date.now() < deadline) {
    last = unwrap(await client.send(requests.getProviderRunRequest(providerRun.id)), 'ProviderRun').provider_run
    if (last?.runtime_mcp_server_url && last?.runtime_mcp_auth_token) return last
    if (last?.state === 'Ended') throw new Error(`provider run ended before exposing runtime MCP: ${JSON.stringify(last)}`)
    await sleep(pollMs)
  }
  throw new Error(`provider run did not expose runtime MCP binding: ${JSON.stringify(last)}`)
}

async function callRuntimeMcp(providerRun, method, params = {}) {
  const response = await fetch(providerRun.runtime_mcp_server_url, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${providerRun.runtime_mcp_auth_token}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: `${method}-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      method,
      params,
    }),
  })
  const text = await response.text()
  let json
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error(`runtime MCP response was not JSON (${response.status}): ${text}`)
  }
  if (!response.ok || json.error) throw new Error(`runtime MCP ${method} failed: ${text}`)
  return json.result
}

async function callRuntimeTool(providerRun, name, args = {}) {
  const result = await callRuntimeMcp(providerRun, 'tools/call', {
    name,
    arguments: args,
  })
  return {
    ok: !result.isError,
    payload: result.structuredContent,
    raw: result,
  }
}

async function listRuntimeToolNames(providerRun) {
  const result = await callRuntimeMcp(providerRun, 'tools/list')
  return (result.tools ?? []).map((tool) => tool.name)
}

async function cleanupSession(kernelUrl, sessionId) {
  if (!sessionId) return
  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const { endSessionRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const client = new LocalIpcClient(kernelUrl)
  try {
    await client.send(endSessionRequest(sessionId)).catch(() => {})
  } finally {
    await client.close().catch(() => {})
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null || child.signalCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null && child.signalCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function writeGroceryApp(workspace) {
  await mkdir(path.join(workspace, 'public'), { recursive: true })
  await writeFile(path.join(workspace, 'package.json'), JSON.stringify({
    scripts: { start: 'node server.mjs' },
    dependencies: {},
    devDependencies: {},
  }, null, 2), 'utf8')
  await writeFile(path.join(workspace, 'server.mjs'), groceryServerSource(), 'utf8')
  await writeFile(path.join(workspace, 'public', 'index.html'), groceryHtmlSource(), 'utf8')
  await writeFile(path.join(workspace, 'public', 'app.js'), groceryAppSource(), 'utf8')
  await writeFile(path.join(workspace, 'public', 'styles.css'), groceryCssSource(), 'utf8')
  await writeFile(path.join(workspace, 'README.md'), [
    '# FreshCart Local Grocery',
    '',
    'Run locally with:',
    '',
    '```sh',
    'PORT=4173 npm start',
    '```',
    '',
    'The app is local-only and uses deterministic in-memory demo data.',
  ].join('\n'), 'utf8')
}

function groceryServerSource() {
  return String.raw`import http from 'node:http'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.dirname(fileURLToPath(import.meta.url))
const port = Number(process.env.PORT || 4173)
const users = new Map()
const sessions = new Map()
const carts = new Map()
const products = [
  { id: 'apple-gala', name: 'Gala Apples', category: 'produce', price: 3.49, stock: 42, detail: 'Crisp Washington apples sold by the bag.' },
  { id: 'sourdough', name: 'Sourdough Boule', category: 'bakery', price: 5.25, stock: 18, detail: 'Naturally leavened loaf baked this morning.' },
  { id: 'olive-oil', name: 'Extra Virgin Olive Oil', category: 'pantry', price: 12.99, stock: 24, detail: 'Cold-pressed olive oil for cooking and salads.' },
  { id: 'greek-yogurt', name: 'Greek Yogurt', category: 'dairy', price: 4.79, stock: 35, detail: 'Plain whole milk yogurt, 32 oz.' },
  { id: 'frozen-peas', name: 'Frozen Peas', category: 'frozen', price: 2.99, stock: 50, detail: 'Sweet peas flash-frozen at peak season.' },
]

const json = (response, status, value, headers = {}) => {
  response.writeHead(status, { 'content-type': 'application/json', ...headers })
  response.end(JSON.stringify(value))
}

const readJson = async (request) => {
  let body = ''
  for await (const chunk of request) body += chunk
  return body ? JSON.parse(body) : {}
}

const tokenFrom = (request) => request.headers.authorization?.replace(/^Bearer\s+/i, '') ?? null
const currentCart = (token) => {
  if (!carts.has(token)) carts.set(token, new Map())
  return carts.get(token)
}
const cartPayload = (token) => [...currentCart(token).entries()].map(([id, quantity]) => ({
  product: products.find((product) => product.id === id),
  quantity,
}))

const server = http.createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://127.0.0.1')
    if (request.method === 'GET' && url.pathname === '/') {
      response.writeHead(200, { 'content-type': 'text/html' })
      response.end(await readFile(path.join(root, 'public', 'index.html'), 'utf8'))
      return
    }
    if (request.method === 'GET' && url.pathname.startsWith('/public/')) {
      const file = path.join(root, url.pathname)
      const type = file.endsWith('.css') ? 'text/css' : file.endsWith('.js') ? 'application/javascript' : 'text/plain'
      response.writeHead(200, { 'content-type': type })
      response.end(await readFile(file, 'utf8'))
      return
    }
    if (request.method === 'POST' && url.pathname === '/api/register') {
      const input = await readJson(request)
      if (!input.email || !input.password) return json(response, 400, { error: 'email and password required' })
      users.set(input.email, { email: input.email, name: input.name || 'Shopper', password: input.password })
      return json(response, 201, { ok: true, email: input.email })
    }
    if (request.method === 'POST' && url.pathname === '/api/login') {
      const input = await readJson(request)
      const user = users.get(input.email)
      if (!user || user.password !== input.password) return json(response, 401, { error: 'invalid login' })
      const token = 'demo-' + Math.random().toString(16).slice(2)
      sessions.set(token, user.email)
      return json(response, 200, { token, user: { email: user.email, name: user.name } })
    }
    if (request.method === 'GET' && url.pathname === '/api/products') {
      return json(response, 200, { products })
    }
    if (request.method === 'GET' && url.pathname.startsWith('/api/products/')) {
      const product = products.find((entry) => entry.id === url.pathname.split('/').pop())
      return product ? json(response, 200, { product }) : json(response, 404, { error: 'not found' })
    }
    if (request.method === 'POST' && url.pathname === '/api/cart') {
      const token = tokenFrom(request)
      if (!sessions.has(token)) return json(response, 401, { error: 'login required' })
      const input = await readJson(request)
      const product = products.find((entry) => entry.id === input.productId)
      if (!product) return json(response, 404, { error: 'product not found' })
      currentCart(token).set(product.id, Number(input.quantity || 1))
      return json(response, 200, { cart: cartPayload(token) })
    }
    if (request.method === 'PATCH' && url.pathname.startsWith('/api/cart/')) {
      const token = tokenFrom(request)
      if (!sessions.has(token)) return json(response, 401, { error: 'login required' })
      const input = await readJson(request)
      currentCart(token).set(url.pathname.split('/').pop(), Number(input.quantity || 1))
      return json(response, 200, { cart: cartPayload(token) })
    }
    if (request.method === 'GET' && url.pathname === '/api/cart') {
      const token = tokenFrom(request)
      if (!sessions.has(token)) return json(response, 401, { error: 'login required' })
      return json(response, 200, { cart: cartPayload(token) })
    }
    if (request.method === 'POST' && url.pathname === '/api/checkout') {
      const token = tokenFrom(request)
      if (!sessions.has(token)) return json(response, 401, { error: 'login required' })
      const cart = cartPayload(token)
      if (cart.length === 0) return json(response, 400, { error: 'cart is empty' })
      const total = cart.reduce((sum, item) => sum + item.product.price * item.quantity, 0)
      currentCart(token).clear()
      return json(response, 200, { confirmation: 'FAKE-' + Date.now(), total: Number(total.toFixed(2)), status: 'confirmed' })
    }
    json(response, 404, { error: 'not found' })
  } catch (error) {
    json(response, 500, { error: error.message })
  }
})

server.listen(port, '127.0.0.1', () => {
  console.log('FreshCart listening on http://127.0.0.1:' + port)
})
`
}

function groceryHtmlSource() {
  return String.raw`<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>FreshCart Local Grocery</title>
  <link rel="stylesheet" href="/public/styles.css">
</head>
<body>
  <main>
    <section class="hero">
      <h1>FreshCart Local Grocery</h1>
      <p>Produce, bakery, pantry, dairy, and frozen staples for deterministic checkout testing.</p>
    </section>
    <section class="auth">
      <input id="email" value="shopper@example.com">
      <input id="password" value="correct-horse" type="password">
      <button id="register">Register</button>
      <button id="login">Login</button>
    </section>
    <section>
      <h2>Categories</h2>
      <div id="products" class="grid"></div>
    </section>
    <section>
      <h2>Cart</h2>
      <div id="cart"></div>
      <button id="checkout">Checkout</button>
      <p id="confirmation"></p>
    </section>
  </main>
  <script src="/public/app.js"></script>
</body>
</html>
`
}

function groceryAppSource() {
  return String.raw`let token = null
const api = async (path, options = {}) => {
  const response = await fetch(path, {
    ...options,
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: 'Bearer ' + token } : {}),
      ...(options.headers || {}),
    },
  })
  const json = await response.json()
  if (!response.ok) throw new Error(json.error || response.statusText)
  return json
}

const renderProducts = async () => {
  const { products } = await api('/api/products')
  document.querySelector('#products').innerHTML = products.map((product) => [
    '<article class="product" data-category="' + product.category + '">',
    '<h3>' + product.name + '</h3>',
    '<p>' + product.category + ' · $' + product.price.toFixed(2) + ' · stock ' + product.stock + '</p>',
    '<p>' + product.detail + '</p>',
    '<button data-add="' + product.id + '">Add</button>',
    '</article>',
  ].join('')).join('')
}

const renderCart = async () => {
  if (!token) return
  const { cart } = await api('/api/cart')
  document.querySelector('#cart').innerHTML = cart.map((item) => (
    '<div>' + item.product.name + ': <input data-qty="' + item.product.id + '" type="number" min="1" value="' + item.quantity + '"></div>'
  )).join('') || 'Cart is empty'
}

document.querySelector('#register').addEventListener('click', async () => {
  await api('/api/register', { method: 'POST', body: JSON.stringify({ email: email.value, password: password.value, name: 'Demo Shopper' }) })
})
document.querySelector('#login').addEventListener('click', async () => {
  const result = await api('/api/login', { method: 'POST', body: JSON.stringify({ email: email.value, password: password.value }) })
  token = result.token
  await renderCart()
})
document.body.addEventListener('click', async (event) => {
  const productId = event.target?.dataset?.add
  if (!productId) return
  await api('/api/cart', { method: 'POST', body: JSON.stringify({ productId, quantity: 1 }) })
  await renderCart()
})
document.body.addEventListener('change', async (event) => {
  const productId = event.target?.dataset?.qty
  if (!productId) return
  await api('/api/cart/' + productId, { method: 'PATCH', body: JSON.stringify({ quantity: Number(event.target.value) }) })
  await renderCart()
})
document.querySelector('#checkout').addEventListener('click', async () => {
  const result = await api('/api/checkout', { method: 'POST' })
  document.querySelector('#confirmation').textContent = result.confirmation + ' confirmed for $' + result.total.toFixed(2)
  await renderCart()
})
renderProducts()
`
}

function groceryCssSource() {
  return String.raw`body { margin: 0; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #f7f8f5; color: #1b1f1a; }
main { max-width: 980px; margin: 0 auto; padding: 32px 20px; }
.hero { border-bottom: 1px solid #d9ded3; margin-bottom: 20px; padding-bottom: 18px; }
h1 { margin: 0 0 8px; font-size: 34px; }
h2 { margin-top: 28px; }
.auth { display: flex; flex-wrap: wrap; gap: 8px; }
input, button { min-height: 38px; border: 1px solid #bbc5b3; border-radius: 6px; padding: 0 10px; font: inherit; }
button { background: #245b3b; color: white; cursor: pointer; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; }
.product { background: white; border: 1px solid #d9ded3; border-radius: 8px; padding: 14px; }
.product h3 { margin-top: 0; }
#cart { background: white; border: 1px solid #d9ded3; border-radius: 8px; padding: 14px; min-height: 42px; }
#confirmation { font-weight: 700; }
`
}

async function waitForHttp(url, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url)
      if (response.ok) return response
      last = `${response.status} ${response.statusText}`
    } catch (error) {
      last = error.message
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${url}: ${last}`)
}

async function runGrocerySmoke(baseUrl) {
  const html = await fetch(baseUrl).then((response) => response.text())
  assert(html.includes('FreshCart Local Grocery'), 'root HTML should identify grocery app')
  const register = await fetch(`${baseUrl}/api/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: 'shopper@example.com', password: 'correct-horse', name: 'Demo Shopper' }),
  }).then((response) => response.json())
  assert(register.ok, 'registration should succeed', register)
  const login = await fetch(`${baseUrl}/api/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: 'shopper@example.com', password: 'correct-horse' }),
  }).then((response) => response.json())
  assert(login.token, 'login should return token', login)
  const auth = { authorization: `Bearer ${login.token}`, 'content-type': 'application/json' }
  const catalog = await fetch(`${baseUrl}/api/products`).then((response) => response.json())
  const categories = new Set((catalog.products ?? []).map((product) => product.category))
  for (const category of ['produce', 'bakery', 'pantry', 'dairy', 'frozen']) {
    assert(categories.has(category), `catalog should include ${category}`, catalog)
  }
  assert(catalog.products.every((product) => product.price > 0 && product.stock > 0), 'products should expose prices and stock', catalog)
  const detail = await fetch(`${baseUrl}/api/products/apple-gala`).then((response) => response.json())
  assert(detail.product?.detail, 'product detail endpoint should return detail text', detail)
  const addCart = await fetch(`${baseUrl}/api/cart`, {
    method: 'POST',
    headers: auth,
    body: JSON.stringify({ productId: 'apple-gala', quantity: 1 }),
  }).then((response) => response.json())
  assert(addCart.cart?.[0]?.quantity === 1, 'cart add should set quantity 1', addCart)
  const updateCart = await fetch(`${baseUrl}/api/cart/apple-gala`, {
    method: 'PATCH',
    headers: auth,
    body: JSON.stringify({ quantity: 3 }),
  }).then((response) => response.json())
  assert(updateCart.cart?.[0]?.quantity === 3, 'cart quantity update should persist', updateCart)
  const checkout = await fetch(`${baseUrl}/api/checkout`, {
    method: 'POST',
    headers: auth,
  }).then((response) => response.json())
  assert(checkout.status === 'confirmed' && checkout.confirmation?.startsWith('FAKE-'), 'checkout should confirm fake purchase', checkout)
  return { categories: [...categories].sort(), confirmation: checkout.confirmation, total: checkout.total }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-grocery-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const scriptsDir = path.join(rootDir, 'scripts')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const appUrl = `http://127.0.0.1:${ports.appPort}`
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `metaagent-grocery-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
  }

  let daemon = null
  let appServer = null
  let client = null
  let sessionId = null
  let succeeded = false
  let failure = null
  let appServerStdout = ''
  let appServerStderr = ''
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(scriptsDir, { recursive: true })
    await initGitWorktree(workspace)

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForDaemon(shellBin, kernelUrl, workspace, env)
    log('daemon-ready', { kernelUrl })

    const setupScript = path.join(scriptsDir, 'setup.arroba')
    await writeFile(setupScript, [
      'set provider dev-stub',
      'set model metaagent-grocery-default',
      'session new --meta $workspace as session',
      'agent list',
    ].join('\n'), 'utf8')
    const setup = await run(process.execPath, [
      shellBin,
      'run',
      setupScript,
      '--kernel-url',
      kernelUrl,
      '--workspace',
      workspace,
      '--worktree',
      workspace,
      '--var',
      `workspace=${workspace}`,
    ], { env })
    if (setup.code !== 0) throw new Error(`setup script failed\nstdout:\n${setup.stdout}\nstderr:\n${setup.stderr}`)
    requireOutput(setup.stdout, /created metaagent session /, 'metaagent session creation')
    sessionId = setup.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'setup script did not bind session id', { stdout: setup.stdout })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-grocery-drill-${Date.now()}`)), 'SessionAttached').attachment
    await client.subscribeToKernelEvents(sessionId, attachment.id)

    let sessionState = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const metaagent = (sessionState.agents ?? []).find((agent) => agent.role === 'meta')
    assert(metaagent, 'session should contain a metaagent', sessionState)
    const metaRun = await launchRuntime(client, requests, sessionId, metaagent.id, 'metaagent-grocery-meta', options.timeoutMs, options.pollMs)
    assert(metaRun.execution_mode === 'plan', 'metaagent provider run must be forced to plan mode', { metaRun })
    assert(metaRun.permission_level === 'required', 'metaagent provider run must require permissions', { metaRun })
    const metaTools = await listRuntimeToolNames(metaRun)
    assert(metaTools.every((tool) => tool.startsWith('arroba.meta.')), 'metaagent runtime tools should be meta-only', { metaTools })
    const deniedRead = await callRuntimeTool(metaRun, 'arroba.read_artifact', { path: 'package.json' })
    assert(!deniedRead.ok, 'direct metaagent workspace read should remain denied', deniedRead.payload)

    const visiblePrompt = await client.send(requests.submitPromptRequest(
      sessionId,
      attachment.id,
      metaagent.id,
      'Create and supervise a workflow that builds a local grocery web app from scratch. Use regular worker agents for implementation and QA; do not implement directly.',
      [],
    ))
    assert(visiblePrompt, 'visible grocery prompt should submit')

    const frontendSpawn = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'agent spawn frontend grocery-frontend-worker' })
    assert(frontendSpawn.ok, 'metaagent should spawn frontend worker', frontendSpawn.payload)
    const qaSpawn = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'agent spawn qa grocery-qa-worker' })
    assert(qaSpawn.ok, 'metaagent should spawn QA worker', qaSpawn.payload)
    const workflowCreate = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'workflow new grocery-flow' })
    assert(workflowCreate.ok, 'metaagent should create grocery workflow', workflowCreate.payload)
    const workflowRef = workflowCreate.payload?.response?.workflow?.id ?? 'grocery-flow'
    const frontendNode = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: `workflow node add ${workflowRef} frontend` })
    assert(frontendNode.ok, 'metaagent should add frontend worker node', frontendNode.payload)
    const frontendNodeId = frontendNode.payload?.response?.node?.id
    assert(frontendNodeId, 'frontend workflow node id should be returned', frontendNode.payload)
    const endpoint = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: `workflow endpoint new ${workflowRef} ${frontendNodeId} default` })
    assert(endpoint.ok, 'metaagent should create grocery workflow endpoint', endpoint.payload)
    const workflowRun = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: `workflow run ${workflowRef} default Build the grocery storefront and smoke tests`,
    })
    assert(workflowRun.ok, 'metaagent should invoke grocery workflow', workflowRun.payload)
    log('delegation-commands-passed')

    sessionState = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const agents = sessionState.agents ?? []
    const frontend = agents.find((agent) => agent.alias === 'frontend')
    const qa = agents.find((agent) => agent.alias === 'qa')
    assert(frontend && qa, 'owned worker agents should exist', { agents })
    assert(frontend.role !== 'meta' && qa.role !== 'meta', 'workers should be regular agents', { frontend, qa })
    const workflow = (sessionState.workflows ?? []).find((entry) => entry.alias === 'grocery-flow' || entry.id === workflowRef)
    assert(workflow, 'grocery workflow should exist', sessionState.workflows)
    assert((workflow.nodes ?? []).every((node) => node.agent_id !== metaagent.id), 'metaagent must not be a workflow node', workflow)
    assert((sessionState.workflow_runs ?? []).length > 0, 'grocery workflow run should be recorded', sessionState.workflow_runs)

    const workerRun = await launchRuntime(client, requests, sessionId, frontend.id, 'grocery-frontend-worker', options.timeoutMs, options.pollMs)
    const promptResult = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'prompt frontend "Build the FreshCart local grocery app files and document how to run them."',
    })
    assert(promptResult.ok, 'metaagent should prompt frontend worker', promptResult.payload)
    await writeGroceryApp(workspace)
    await client.send(requests.appendNativeProviderOutputRequest(
      sessionId,
      attachment.id,
      workerRun.id,
      'provider_tool',
      JSON.stringify({
        tool: 'workspace_write',
        status: 'completed',
        output: 'Created FreshCart grocery app files: package.json, server.mjs, public/index.html, public/app.js, public/styles.css, README.md',
      }),
      'metaagent-grocery-worker-output',
    ))
    await client.send(requests.completePromptRequest(sessionId)).catch((error) => {
      if (!String(error?.message ?? error).includes('has no active prompt')) throw error
    })

    const expectedFiles = ['package.json', 'server.mjs', 'public/index.html', 'public/app.js', 'public/styles.css', 'README.md']
    for (const file of expectedFiles) {
      const exists = await stat(path.join(workspace, file)).then((info) => info.isFile()).catch(() => false)
      assert(exists, `generated app file should exist: ${file}`)
    }
    const packageJson = JSON.parse(await readFile(path.join(workspace, 'package.json'), 'utf8'))
    assert(packageJson.scripts?.start === 'node server.mjs', 'generated app should document start command in package.json', packageJson)
    log('app-files-generated', { expectedFiles })

    appServer = spawn(process.execPath, ['server.mjs'], {
      cwd: workspace,
      env: { ...process.env, PORT: String(ports.appPort) },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    appServer.stdout.on('data', (chunk) => { appServerStdout += chunk.toString() })
    appServer.stderr.on('data', (chunk) => { appServerStderr += chunk.toString() })
    await waitForHttp(appUrl, options.timeoutMs, options.pollMs)
    const smoke = await runGrocerySmoke(appUrl)
    log('grocery-smoke-passed', smoke)

    const postSmokeDenied = await callRuntimeTool(metaRun, 'arroba.read_artifact', { path: 'server.mjs' })
    assert(!postSmokeDenied.ok, 'direct metaagent tool calls should remain denied after app generation', postSmokeDenied.payload)

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'metaagent-grocery-drill',
      kernelUrl,
      appUrl,
      sessionId,
      metaagentId: metaagent.id,
      workerIds: [frontend.id, qa.id],
      workflowId: workflow.id,
      generatedFiles: expectedFiles,
      smoke,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    await cleanupSession(kernelUrl, sessionId)
    await terminateChild(appServer)
    await terminateChild(daemon)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'metaagent-grocery',
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        kernelUrl,
        sessionId,
        workspace,
        appUrl,
        appServerStdout,
        appServerStderr,
      },
      log,
    })
  }
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})
