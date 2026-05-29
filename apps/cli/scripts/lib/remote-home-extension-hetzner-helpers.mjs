import { spawn } from "node:child_process"
import path from "node:path"
import { waitForTcpPort } from "./drill-runtime-helpers.mjs"
import { remoteEnvCommand, runHetznerCommand, shellQuote, sshArgs } from "./native-tui-remote-execution.mjs"

export async function ensureRemoteHomeExtensionHetznerWorkspace(options, {
  remoteRoot,
  workerWorktree,
}) {
  await runHetznerCommand(options, [
    `test -x ${shellQuote(path.posix.join(options.hetznerRepo, "apps/kernel/target/debug/arroba-kernel"))}`,
    `test -x ${shellQuote(path.posix.join(options.hetznerRepo, "apps/relay/target/debug/arroba-relay"))}`,
    `mkdir -p ${shellQuote(remoteRoot)} ${shellQuote(workerWorktree)}`,
  ].join("; "))
}

export async function stopRemoteHomeExtensionHetznerRelay(options, port) {
  await runHetznerCommand(options, `pkill -f ${shellQuote(`ARROBA_RELAY_PORT=${port}`)} 2>/dev/null || true`).catch(() => {})
  await runHetznerCommand(options, `fuser -k ${port}/tcp 2>/dev/null || true`).catch(() => {})
}

export async function removeRemoteHomeExtensionHetznerRoot(options, remoteRoot) {
  await runHetznerCommand(options, `rm -rf ${shellQuote(remoteRoot)}`).catch(() => {})
}

export async function startRemoteHomeExtensionHetznerRelay({
  options,
  relayPort,
  workerMcpPort,
  sharedRelayToken,
  collab,
  issuer,
  secret,
}) {
  await stopRemoteHomeExtensionHetznerRelay(options, relayPort)
  const relay = spawn("ssh", sshArgs(options, remoteEnvCommand({
    ARROBA_REMOTE_REPO: options.hetznerRepo,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(relayPort),
    ARROBA_RELAY_TOKEN: sharedRelayToken,
    ...(collab ? {
      ARROBA_RELAY_SCOPED_ISSUER: issuer,
      ARROBA_RELAY_SCOPED_HMAC_SECRET: secret,
    } : {}),
  }, "./apps/relay/target/debug/arroba-relay")), { stdio: ["ignore", "ignore", "inherit"] })
  const tunnel = spawn("ssh", [
    "-i",
    options.hetznerKey,
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-N",
    "-L",
    `127.0.0.1:${relayPort}:127.0.0.1:${relayPort}`,
    "-L",
    `127.0.0.1:${workerMcpPort}:127.0.0.1:${workerMcpPort}`,
    options.hetznerHost,
  ], { stdio: ["ignore", "ignore", "inherit"] })
  await waitForTcpPort(relayPort, "127.0.0.1", 30_000)
  return { relay, tunnel }
}

export function spawnRemoteHomeExtensionHetznerWorker({
  options,
  remoteRoot,
  workerWorktree,
  relayPort,
  workerRelayToken,
  workerDaemonId,
  workerMachineId,
  workerAlias,
  workerKernelPort,
  workerMcpPort,
}) {
  return spawn("ssh", sshArgs(options, remoteEnvCommand({
    ARROBA_REMOTE_REPO: options.hetznerRepo,
    PATH: "/root/.cargo/bin:/root/.bun/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    HOME: path.posix.join(remoteRoot, "worker-home"),
    CODEX_HOME: "/root/.codex",
    OPENCODE_CONFIG_DIR: "/root/.config/opencode",
    XDG_CONFIG_HOME: path.posix.join(remoteRoot, "worker-config"),
    XDG_STATE_HOME: path.posix.join(remoteRoot, "worker-state"),
    ARROBA_KERNEL_PORT: String(workerKernelPort),
    ARROBA_MCP_PORT: String(workerMcpPort),
    ARROBA_OPENCODE_PORT: String(workerKernelPort + 2000),
    ARROBA_CODEX_PORT: String(workerKernelPort + 2001),
    ARROBA_RELAY_URL: `ws://127.0.0.1:${relayPort}`,
    ARROBA_RELAY_TOKEN: workerRelayToken,
    ARROBA_DAEMON_ID: workerDaemonId,
    ARROBA_DAEMON_ALIAS: "worker",
    ARROBA_MACHINE_ID: workerMachineId,
    ARROBA_MACHINE_ALIAS: workerAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: "1",
    ARROBA_DAEMON_SOCKET: path.posix.join(remoteRoot, "worker.sock"),
    ARROBA_SESSION_HISTORY_DIR: path.posix.join(remoteRoot, "worker-history"),
    ARROBA_CAPABILITY_ISOLATION_ROOT: path.posix.join(remoteRoot, "worker-capabilities"),
  }, `mkdir -p ${shellQuote(remoteRoot)} ${shellQuote(workerWorktree)} && ./apps/kernel/target/debug/arroba-kernel`)), {
    stdio: ["ignore", "ignore", "inherit"],
  })
}
