// OF-32: a second kernel that reaches the drill kernel through a private scoped relay, so a follower on that
// kernel (the real `chariox events` command, `--kernel <name>`) sees the drill Room's process events through the gateway.
import { spawn } from "node:child_process"
import { createHmac } from "node:crypto"
import { mkdirSync, openSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join } from "node:path"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"

const freePort = () => new Promise(done => { const s = createServer().listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => done(p)) }) })

/** Starts the relay and the second kernel, pairs it with the drill kernel and returns its client, environment and the name it calls the drill kernel. */
export async function startReach({ relayBinary, kernelBinary, dir, evidence, drillEnv, drillRpc, passkey, own, until, note }) {
  const secret = "reach-" + Math.random().toString(36).slice(2) + Date.now()
  const issuer = `process-admission-reach-${Date.now()}`
  const token = (subject, kind, actions, targets = null, machineId = null) => {
    const now = Date.now()
    const claims = { issuer, subject, subject_kind: kind, realm_id: "local-dev", allowed_actions: actions, allowed_targets: targets, issued_at_ms: now, expires_at_ms: now + 2 * 60 * 60_000, token_id: `${subject}-${now}`,
      account_id: "local-dev-account", organization_id: null, user_id: "local", device_id: null, machine_id: kind === "kernel" ? machineId : null, client_id: kind === "client" ? subject : null, session_id: null, public_key_thumbprint: null, entitlements_version: "process-admission-reach" }
    const payload = Buffer.from(JSON.stringify(claims)).toString("base64url")
    return `chariox-scoped-v1.${payload}.${createHmac("sha256", secret).update(payload).digest("base64url")}`
  }
  const relayPort = await freePort()
  const relayLog = openSync(join(evidence, "reach-relay.log"), "w", 0o600)
  own(spawn(relayBinary, [], { env: { PATH: process.env.PATH, HOME: dir, CHARIOX_HOME: join(dir, "relay"), CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, stdio: ["ignore", relayLog, relayLog] }))
  const root = join(dir, "reach-kernel")
  for (const sub of ["home", "chariox", "tmp", "work", "logs", "history"]) mkdirSync(join(root, sub), { recursive: true, mode: 0o700 })
  const alias = "reach-kernel"
  writeFileSync(join(root, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/chariox/vault.json"\nunlock_policy = "kernel_init"\n`, { mode: 0o600 })
  const kernelPort = await freePort()
  const env = { PATH: drillEnv.PATH, LANG: "C.UTF-8", NODE_OPTIONS: "--max-old-space-size=1024", IS_SANDBOX: "1", SHELL: "/bin/bash", HOME: join(root, "home"), TMPDIR: join(root, "tmp"),
    XDG_CONFIG_HOME: join(root, "xdg-config"), XDG_CACHE_HOME: join(root, "xdg-cache"), XDG_STATE_HOME: join(root, "xdg-state"), CHARIOX_HOME: join(root, "chariox"), CHARIOX_LOG_DIR: join(root, "logs"),
    CHARIOX_SESSION_HISTORY_DIR: join(root, "history"), CHARIOX_DAEMON_SOCKET: join(root, "d.sock"), CHARIOX_DAEMON_ALIAS: alias, CHARIOX_MACHINE_ALIAS: alias, CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(await freePort()), CHARIOX_OPENCODE_PORT: String(await freePort()), CHARIOX_CODEX_PORT: String(await freePort()) }
  const kernelLog = openSync(join(evidence, "reach-kernel.log"), "w", 0o600)
  const kernel = own(spawn(kernelBinary, [], { env, stdio: ["ignore", kernelLog, kernelLog] }))
  const client = new LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`, { timeoutMs: 15000, localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  const rpc = async request => { const r = await client.send(request); if (r.Error) throw new Error(`reach refused ${Object.keys(request)[0]}: ${JSON.stringify(r.Error)}`); return r }
  const statusOf = async send => until(async () => { try { return (await send({ RelayStatus: null })).RelayStatus.status } catch { return null } }, "reach kernel ready", 60000)
  const reachStatus = await statusOf(rpc)
  const drillStatus = (await drillRpc({ RelayStatus: null })).RelayStatus.status
  const actions = ["daemon_register", "daemon_heartbeat", "client_metadata_read", "packet_route", "peer_request", "peer_event"]
  for (const [send, status] of [[rpc, reachStatus], [drillRpc, drillStatus]]) {
    await send({ ConfigureRelay: { relay_url: `ws://127.0.0.1:${relayPort}`, relay_token: token(status.daemon_id, "kernel", actions, null, status.machine_id) } })
    await until(async () => (await send({ RelayStatus: null })).RelayStatus.status.connected, "relay connected")
  }
  // The reach kernel's Vault must be open to approve its own follower's grant.
  const made = (await rpc({ CreateSession: { workspace_id: join(root, "work"), worktree_id: join(root, "work"), alias: "reach", no_subscribe: true } })).SessionCreated
  const pending = rpc({ ManageCredentialVault: { session_id: made.session.id, agent_id: made.agent.id } }); pending.catch(() => {})
  const prompt = await until(async () => (await rpc({ GetSessionState: { session_id: made.session.id } })).SessionState.session.active_interactions?.find(x => x.id.startsWith("vault-unlock-")), "reach vault prompt")
  await rpc({ RespondToInteraction: { session_id: made.session.id, interaction_id: prompt.id, choice_id: "passphrase", custom_reply: passkey } })
  await pending
  client.onKernelEvent(async event => {
    if (event.event !== "passkey_prompts_changed") return
    for (const p of event.prompts || []) if (p.kind === "access_grant") {
      note("reach-approve-grant", { requester: p.requester?.executable })
      await rpc({ RespondToInteraction: { session_id: p.session_id || "kernel-access", interaction_id: p.interaction_id, choice_id: "approve", passkey } }).catch(() => note("reach-approval-failed"))
    }
  })
  const pairing = (await drillRpc({ CreateTerminalPairingLink: { terminal_type: "cli", alias: null, expires_in_ms: 3_600_000 } })).TerminalPairingLinkCreated.pairing
  const joined = await rpc({ JoinKernelLink: { link: pairing.pairing_link, relay_token: token("reach-client", "client", ["client_connect", "client_metadata_read", "packet_route"], [drillStatus.daemon_id]) } })
  if (!joined.GatewayKernelJoined) throw new Error("reach pairing failed")
  note("reach-paired", { target: joined.GatewayKernelJoined.kernel.name })
  return { env, rpc, target: joined.GatewayKernelJoined.kernel.name, kernelPid: kernel.pid }
}
