// Real kernels, relay, linked provider account and built TUI. No credential-file access.
import { spawn } from 'node:child_process'
import { mkdir, open, writeFile } from 'node:fs/promises'
import path from 'node:path'
import net from 'node:net'
import readline from 'node:readline'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../../packages/kernel-client/dist/ipc-requests.js'

const root = process.env.CREDCOPIES_RUNTIME_ROOT
const bin = process.env.CREDCOPIES_BINARY_ROOT
const evidence = process.env.CREDCOPIES_EVIDENCE_ROOT
if (!root?.startsWith('/') || !bin?.startsWith('/') || !evidence?.startsWith('/')) throw new Error('explicit absolute runtime, binary and evidence roots required')
await mkdir(root, { recursive: true, mode: 0o700 })
await mkdir(evidence, { recursive: true })
const provider = process.env.CREDCOPIES_PROVIDER ?? 'codex'
const receiverOnly = process.env.CREDCOPIES_RECEIVER_ONLY === '1'
const model = process.env.CREDCOPIES_MODEL ?? 'gpt-6.1-sol'
const nonce = Date.now()
const ports = 55000 + Math.floor(Math.random() * 900)
const kernelIds = { source: process.env.CREDCOPIES_SOURCE_KERNEL ?? `credcopies-source-${nonce}`, worker: process.env.CREDCOPIES_WORKER_KERNEL ?? `credcopies-worker-${nonce}` }
const children = []
const clients = []
const logs = []
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const safeEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('CHARIOX_')))
const stop = async () => {
  for (const client of clients) await client.close().catch(() => {})
  for (const child of children.reverse()) {
    if (child.exitCode !== null) continue
    child.kill('SIGTERM')
    await Promise.race([new Promise(resolve => child.once('exit', resolve)), sleep(3000)])
    if (child.exitCode === null) child.kill('SIGKILL')
  }
  for (const log of logs) await log.close()
}
const variant = (response, name) => {
  if (!response?.[name]) throw new Error(`expected ${name}; got ${Object.keys(response ?? {}).join(', ')}`)
  return response[name]
}
const poll = async (fn, timeout = 45000) => {
  const deadline = Date.now() + timeout
  let last
  while (Date.now() < deadline) {
    try { const value = await fn(); if (value) return value } catch (error) { last = error }
    await sleep(250)
  }
  throw last ?? new Error('live drill wait expired')
}
const launch = async (name, command, env) => {
  const log = await open(path.join(root, `${name}.log`), 'a', 0o600)
  logs.push(log)
  const child = spawn(command, [], { cwd: process.cwd(), env, stdio: ['ignore', log.fd, log.fd] })
  children.push(child)
  return child
}
process.on('SIGTERM', async () => { await stop(); process.exit(0) })
process.on('SIGINT', async () => { await stop(); process.exit(0) })
let home, worker, sessionId, attachmentId, agentId, accountId, workerMachineId
const snapshot = async stage => {
  const profiles = variant(await home.send({ ListProviderAccountProfiles: { provider } }), 'ProviderAccountProfilesListed').profiles
  const session = variant(await home.send(requests.getSessionStateRequest(sessionId)), 'SessionState').session
  const agent = session.agents.find(agent => agent.id === agentId)
  // Human-only login challenges and provider payloads never enter evidence.
  const receipt = { stage, protocol: 455, kernelIds, workerMachineId, provider, accountId,
    copies: profiles.find(profile => profile.profile_id === accountId)?.materializations ?? [],
    agentState: agent?.state,
    interactions: (session.active_interactions ?? []).filter(interaction => interaction.agent_id === agentId).map(interaction => ({ id: interaction.id, title: interaction.title, hasOfficialLogin: !!interaction.provider_login })),
    promptState: Object.fromEntries(Object.entries(session.prompt_states ?? {}).filter(([id]) => id === agentId).map(([id, state]) => [id, { active: state.active_prompt?.id ?? null, queued: state.queued_prompts?.map(prompt => prompt.id) ?? [] }])) }
  await writeFile(path.join(evidence, `${stage}.json`), JSON.stringify(receipt, null, 2))
  console.log(JSON.stringify(receipt))
  return session
}
try {
  await launch('relay', path.join(bin, 'chariox-relay'), { ...safeEnv, CHARIOX_RELAY_HOST: '127.0.0.1', CHARIOX_RELAY_PORT: String(ports), CHARIOX_RELAY_TOKEN: `credcopies-${nonce}` })
  for (const [index, role] of (receiverOnly ? ['worker'] : ['source', 'worker']).entries()) {
    const homePath = path.join(root, role)
    await mkdir(homePath, { recursive: true, mode: 0o700 })
    const env = { ...safeEnv, CHARIOX_HOME: homePath, CHARIOX_DAEMON_ID: kernelIds[role], CHARIOX_DAEMON_ALIAS: role,
      CHARIOX_DAEMON_SOCKET: `/tmp/chariox-${process.getuid()}/credcopies-${role}-${nonce}.sock`,
      CHARIOX_KERNEL_PORT: String(ports + 1000 + index), CHARIOX_MCP_PORT: String(ports + 2000 + index),
      CHARIOX_CODEX_PORT: String(ports + 3000 + index), CHARIOX_OPENCODE_PORT: String(ports + 4000 + index),
      CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports}`, CHARIOX_RELAY_TOKEN: `credcopies-${nonce}`, CHARIOX_ACCEPT_REMOTE_LEASES: role === 'worker' ? '1' : '0',
      CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: '1' }
    if (role === 'worker') {
      for (const key of ['CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_STATE_HOME', 'OPENCODE_CONFIG_DIR', 'OPENAI_API_KEY', 'ANTHROPIC_API_KEY']) delete env[key]
      env.HOME = path.join(homePath, 'provider-home')
      await mkdir(env.HOME, { recursive: true, mode: 0o700 })
    }
    await launch(role, path.join(bin, 'chariox-kernel'), env)
    await poll(() => new Promise(resolve => { const socket = net.createConnection({host:'127.0.0.1',port:Number(env.CHARIOX_KERNEL_PORT)}); socket.once('error',()=>resolve(false)); socket.once('connect',()=>{socket.destroy();resolve(true)}) }))
    await sleep(250)
    const client = new LocalIpcClient(`ws://127.0.0.1:${env.CHARIOX_KERNEL_PORT}/kernel`, {localAuthEnvironment:env, controlRequestRetryDeadlineMs:1000})
    clients.push(client)
    await poll(async () => variant(await client.send({ ListProviderAccountProfiles: { provider } }), 'ProviderAccountProfilesListed'))
    if (role === 'source') home = client; else worker = client
  }
  if (receiverOnly) home = worker
  await home.send(requests.setUserConfigValueRequest('providers.workspace_live_sync', 'off'))
  await worker.send(requests.setUserConfigValueRequest('providers.workspace_live_sync', 'off'))
  // Chariox links the existing official login; the drill never opens an auth file.
  const imported = receiverOnly ? variant(await worker.send({GetProviderAccountProfile:{provider,account_profile:process.env.CREDCOPIES_RECEIVING_ACCOUNT}}),'ProviderAccountProfile').profile : variant(await home.send({ ImportNativeProviderAccountProfile: { provider } }), 'ProviderAccountProfile').profile
  accountId = imported.profile_id
  const refreshed = receiverOnly || process.env.CREDCOPIES_RESUME === '1' ? imported : variant(await home.send({ RefreshProviderAccountProfile: { provider, account_profile: accountId } }), 'ProviderAccountProfile').profile
  if (!receiverOnly && refreshed.auth_state !== 'authenticated') throw new Error(`${provider} has no authenticated linked login`)
  if (!receiverOnly) await poll(async () => {
    const response = await home.send(requests.listRemoteMachinesRequest())
    const machines = response.RemoteMachinesListed?.machines ?? []
    for (const machine of machines.filter(machine => machine.online)) {
      const kernels = variant(await home.send(requests.listRemoteMachineKernelsRequest(machine.machine_id)), 'RemoteMachineKernelsListed').kernels
      if (kernels.some(kernel => kernel.kernel_id === kernelIds.worker)) {
        workerMachineId = machine.machine_id
        await home.send(requests.approveRemoteMachineRequest(workerMachineId))
        return machine
      }
    }
    return null
  })
  const workspace = process.cwd()
  const session = variant(await home.send(requests.createSessionRequest(workspace, workspace, `credcopies-${provider}-${nonce}`)), 'SessionCreated').session
  sessionId = session.id
  const attachment = variant(await home.send(requests.attachToSessionRequest(sessionId, `credcopies-drill-${nonce}`)), 'SessionAttached').attachment
  attachmentId = attachment.id
  const spawned = variant(await home.send(requests.spawnAgentRequest(sessionId, provider, 'copied-login', model, workspace, 'high', 'build', 'required', receiverOnly ? undefined : kernelIds.worker, undefined, undefined, accountId)), 'AgentSpawned')
  agentId = spawned.agent.id
  await home.send(requests.focusAgentRequest(sessionId, agentId))
  await snapshot('copied')
  const state = { root, sourceHome: path.join(root, receiverOnly ? "worker" : "source"), evidence, kernelIds, provider, model, sessionId, attachmentId, agentId, accountId, sourceSocket: home.socketPath, workerSocket: worker.socketPath, workerMachineId }
  await writeFile(path.join(root, 'live.json'), JSON.stringify(state, null, 2))
  console.log('READY: commands are prompt, logout, observe, queued, login, stop. Login requires a real human response.')
  for await (const command of readline.createInterface({ input: process.stdin })) {
    try {
      if (command === 'stop') break
      if (command === 'prompt' || command === 'queued') attachmentId = variant(await home.send(requests.attachToSessionRequest(sessionId, `credcopies-drill-${nonce}`)), 'SessionAttached').attachment.id
      if (command === 'prompt') await home.send(requests.submitPromptRequest(sessionId, attachmentId, agentId, 'Reply with exactly CREDCOPIES_LIVE_OK. Do not call tools.', []))
      if (command === 'logout') {
        // The official CLI performs logout on the receiver; only its profile path comes from the registry metadata.
        const profileRoot = path.join(root, 'worker', 'state', 'provider-accounts', 'local', provider, accountId)
        const env = { ...safeEnv, HOME: path.join(root, 'worker', 'provider-home') }
        let args
        if (provider === 'codex') {
          env.CODEX_HOME = path.join(profileRoot, 'codex')
          args = ['logout']
        } else if (provider === 'opencode') {
          const service = process.env.CREDCOPIES_SERVICE
          if (!service) throw new Error('OpenCode logout requires the explicit receiving service id in CREDCOPIES_SERVICE')
          for (const [key, directory] of Object.entries({ XDG_DATA_HOME: 'data', XDG_CONFIG_HOME: 'config', XDG_STATE_HOME: 'state', XDG_CACHE_HOME: 'cache', OPENCODE_CONFIG_DIR: 'config/opencode' })) env[key] = path.join(profileRoot, directory)
          args = ['auth', 'logout', service]
        } else {
          throw new Error('this logout drill supports Codex and OpenCode')
        }
        const child = spawn(provider, args, { env, stdio: ['ignore', 'ignore', 'ignore'] })
        children.push(child)
        await new Promise((resolve, reject) => child.once('exit', code => code === 0 ? resolve() : reject(new Error('official receiving-copy logout failed'))))
        await worker.send({ RefreshProviderAccountProfile: { provider, account_profile: accountId } })
      }
      if (command === 'queued') {
        await home.send(requests.submitPromptRequest(sessionId, attachmentId, agentId, 'Reply with exactly CREDCOPIES_AFTER_LOGIN. Do not call tools.', []))
        await home.send(requests.submitPromptRequest(sessionId, attachmentId, agentId, 'Reply with exactly CREDCOPIES_QUEUE_RESUMED. Do not call tools.', []))
      }
      if (command === 'login') {
        const session = variant(await home.send(requests.getSessionStateRequest(sessionId)), 'SessionState').session
        const interaction = session.active_interactions?.find(interaction => interaction.agent_id === agentId && interaction.id.startsWith('provider-auth-recovery:'))
        if (!interaction) throw new Error('no receiving login request')
        await home.send(requests.respondToInteractionRequest(sessionId, interaction.id, 'login'))
      }
      await snapshot(`${command}-${Date.now()}`)
    } catch (error) { console.log(JSON.stringify({ command, error: error.message })) }
  }
} finally { await stop() }
