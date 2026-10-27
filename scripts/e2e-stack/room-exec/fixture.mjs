// MP-08 / MP-09 / MP-10 / MP-11: disposable kernels and scoped encrypted relay.
import { spawn } from 'node:child_process'
import { createHmac, randomBytes } from 'node:crypto'
import { mkdir, open, writeFile, rm, readdir, copyFile } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'

async function port() {
  const server = net.createServer()
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve) })
  const value = server.address().port
  await new Promise(resolve => server.close(resolve))
  return value
}

export async function until(read, label, timeout = 30_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    const value = await read()
    if (value) return value
    await delay(50)
  }
  throw new Error(`timed out: ${label}`)
}

export class Fixture {
  children = []
  clients = []
  envs = {}
  ports = {}
  ids = {}
  passphrase = randomBytes(24).toString('hex')
  secret = randomBytes(32).toString('base64url')
  issuer = `room-exec-${Date.now()}`
  constructor(options) { Object.assign(this, options) }

  token(subject, kind, actions, targets = null, machineId = null) {
    const now = Date.now()
    const claims = {
      issuer: this.issuer, subject, subject_kind: kind, realm_id: 'local-dev',
      allowed_actions: actions, allowed_targets: targets, issued_at_ms: now,
      expires_at_ms: now + 2 * 60 * 60_000, token_id: `${subject}-${now}`,
      account_id: 'local-dev-account', organization_id: null, user_id: 'local',
      device_id: null, machine_id: kind === 'kernel' ? machineId : null,
      client_id: kind === 'client' ? subject : null, session_id: null,
      public_key_thumbprint: null, entitlements_version: 'room-exec-drill',
    }
    const payload = Buffer.from(JSON.stringify(claims)).toString('base64url')
    return `chariox-scoped-v1.${payload}.${createHmac('sha256', this.secret).update(payload).digest('base64url')}`
  }

  async launch(name, executable, env, args = []) {
    const log = await open(path.join(this.evidence, `${name}.log`), 'w', 0o600)
    const child = spawn(executable, args, { env, stdio: ['ignore', log.fd, log.fd] })
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned process PID')
    child.done = new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('exit', (code, signal) => resolve({ code, signal }))
    })
    child.done.catch(() => undefined)
    this.children.push({ name, child, log })
    return child
  }

  async start() {
    await mkdir(this.home, { mode: 0o700 })
    await mkdir(this.evidence, { recursive: true })
    this.ports.relay = await port()
    await this.launch('relay', this.relayBinary, {
      PATH: process.env.PATH, HOME: this.home, CHARIOX_HOME: path.join(this.home, 'relay'),
      CHARIOX_RELAY_HOST: '127.0.0.1', CHARIOX_RELAY_PORT: String(this.ports.relay),
      CHARIOX_RELAY_SCOPED_ISSUER: this.issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: this.secret,
    })
    for (const name of ['A', 'B']) {
      const root = path.join(this.home, name)
      for (const directory of ['home', 'chariox', 'tmp', 'work', 'logs']) await mkdir(path.join(root, directory), { recursive: true, mode: 0o700 })
      const home = path.join(root, 'chariox')
      await writeFile(path.join(home, 'config.toml'), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${path.join(home, 'vault.json')}"\nunlock_policy = "kernel_init"\n`, { mode: 0o600 })
      this.ports[name] = await port()
      this.envs[name] = {
        PATH: process.env.PATH, HOME: path.join(root, 'home'), SHELL: '/bin/bash',
        CHARIOX_HOME: home, TMPDIR: path.join(root, 'tmp'),
        XDG_CONFIG_HOME: path.join(root, 'xdg-config'), XDG_STATE_HOME: path.join(root, 'xdg-state'),
        XDG_CACHE_HOME: path.join(root, 'xdg-cache'), CHARIOX_LOG_DIR: path.join(root, 'logs'),
        CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
        CHARIOX_DAEMON_SOCKET: path.join(root, 'd.sock'), CHARIOX_DAEMON_ALIAS: `extexec-${name}`,
        CHARIOX_MACHINE_ALIAS: `extexec-${name}`, CHARIOX_KERNEL_PORT: String(this.ports[name]),
        CHARIOX_MCP_PORT: String(await port()), CHARIOX_OPENCODE_PORT: String(await port()),
        CHARIOX_CODEX_PORT: String(await port()), NODE_OPTIONS: '--max-old-space-size=1024',
      }
      await this.launch(`kernel-${name}`, this.kernelBinary, this.envs[name])
      const client = new LocalIpcClient(`ws://127.0.0.1:${this.ports[name]}/kernel`, {
        localAuthEnvironment: this.envs[name], controlRequestRetryDeadlineMs: 0,
      })
      this.clients.push(client)
      this[name] = client
      const status = await until(async () => {
        try { return (await client.send({ RelayStatus: null })).RelayStatus.status } catch { return null }
      }, `kernel ${name} startup`, 60_000)
      this.ids[name] = status.daemon_id
      await client.send({ ConfigureRelay: { relay_url: `ws://127.0.0.1:${this.ports.relay}`, relay_token: this.token(status.daemon_id, 'kernel', [
        'daemon_register', 'daemon_heartbeat', 'client_metadata_read', 'packet_route', 'peer_request', 'peer_event',
      ], null, status.machine_id) } })
      await until(async () => (await client.send({ RelayStatus: null })).RelayStatus.status.connected, `kernel ${name} relay`)
    }
    await this.unlock()
    const pairing = (await this.B.send({ CreateTerminalPairingLink: { terminal_type: 'cli', alias: null, expires_in_ms: 3_600_000 } })).TerminalPairingLinkCreated.pairing
    const joined = await this.A.send({ JoinKernelLink: { link: pairing.pairing_link,
      relay_token: this.token('extexec-client', 'client', ['client_connect', 'client_metadata_read', 'packet_route'], [this.ids.B]),
    } })
    if (!joined.GatewayKernelJoined) throw new Error('gateway pairing failed')
    this.target = joined.GatewayKernelJoined.kernel.name
    this.unix = new LocalIpcClient(`ws+unix://${this.envs.A.CHARIOX_DAEMON_SOCKET}`, { controlRequestRetryDeadlineMs: 0 })
    this.clients.push(this.unix)
    this.prompts = []
    this.A.onKernelEvent(event => {
      if (event.event === 'passkey_prompts_changed') this.prompts = event.prompts
    })
    await this.A.subscribeToWaitingRoomInventory()
  }

  async unlock() {
    const work = path.join(this.home, 'A/work')
    const made = (await this.A.send({ CreateSession: { workspace_id: work, worktree_id: work, alias: null, no_subscribe: true } })).SessionCreated
    const pending = this.A.send({ ManageCredentialVault: { session_id: made.session.id, agent_id: made.agent.id } })
    pending.catch(() => undefined)
    const prompt = await until(async () => {
      const state = await this.A.send({ GetSessionState: { session_id: made.session.id } })
      return state.SessionState.session.active_interactions?.find(item => item.id.startsWith('vault-unlock-'))
    }, 'disposable Vault setup')
    await this.A.send({ RespondToInteraction: { session_id: made.session.id, interaction_id: prompt.id, choice_id: 'passphrase', custom_reply: this.passphrase } })
    await pending
    await this.A.send({ EndSession: { session_id: made.session.id } })
  }

  async grant() {
    const pending = this.unix.send({ RequestKernelAccess: { holder_pid: process.pid, term: 'until_holder_exits' } })
    pending.catch(() => undefined)
    const prompt = await until(() => this.prompts.find(item => item.kind === 'access_grant'), 'disposable access decision')
    await this.A.send({ RespondToInteraction: { session_id: prompt.session_id, interaction_id: prompt.interaction_id,
      choice_id: prompt.approve_choice_id, passkey: this.passphrase } })
    const reply = await pending
    if (!reply.KernelAccessGranted) throw new Error('access grant was not issued')
    return reply.KernelAccessGranted.grant.grant_id
  }

  async cleanup() {
    for (const client of this.clients) await client.close().catch(() => undefined)
    const cleanup = []
    for (const { name, child, log } of this.children.reverse()) {
      if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned process PID')
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM')
      let timer
      try {
        await Promise.race([child.done, new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error(`cleanup timeout: ${name}`)), 15_000)
        })])
      } catch {
        if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
        await child.done
      } finally { clearTimeout(timer); await log.close() }
      cleanup.push({ name, pid: child.pid, code: child.exitCode, signal: child.signalCode })
    }
    for (const name of ['A', 'B']) {
      const logRoot = path.join(this.home, name, 'logs')
      for (const file of await readdir(logRoot).catch(() => [])) {
        if (file.endsWith('.ndjson')) await copyFile(path.join(logRoot, file), path.join(this.evidence, `${name}-${file}`))
      }
    }
    await rm(this.home, { recursive: true, force: true })
    await writeFile(path.join(this.evidence, 'cleanup.json'), JSON.stringify({ processes: cleanup, state_removed: this.home }, null, 2))
  }
}
