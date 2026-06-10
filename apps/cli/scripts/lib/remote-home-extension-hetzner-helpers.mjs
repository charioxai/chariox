import { spawn } from "node:child_process"
import path from "node:path"
import { waitForTcpPort } from "./drill-runtime-helpers.mjs"
import {
  assertHetznerArrobaBinaries,
  remoteEnvCommand,
  runHetznerCommand,
  shellQuote,
  sshArgs,
} from "./native-tui-remote-execution.mjs"

export async function ensureRemoteHomeExtensionHetznerWorkspace(options, {
  remoteRoot,
  workerWorktree,
}) {
  await assertHetznerArrobaBinaries(options)
  await runHetznerCommand(options, [
    `mkdir -p ${shellQuote(remoteRoot)} ${shellQuote(workerWorktree)}`,
  ].join("; "))
}

async function assertRemoteRelayPortFree(options, port) {
  await runHetznerCommand(options, [
    "python3 -c",
    shellQuote([
      "import socket, sys",
      "port = int(sys.argv[1])",
      "sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)",
      "sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)",
      "try:",
      "    sock.bind(('127.0.0.1', port))",
      "except OSError as exc:",
      "    print(f'Hetzner relay port {port} is already in use; choose another run or wait for the owner to finish: {exc}', file=sys.stderr)",
      "    raise SystemExit(17)",
      "finally:",
      "    sock.close()",
    ].join("\n")),
    String(port),
  ].join(" "))
}

export async function stopRemoteHomeExtensionHetznerRelay(options, remoteRoot) {
  const pidFile = path.posix.join(remoteRoot, "relay.pid")
  await runHetznerCommand(options, [
    `if test -f ${shellQuote(pidFile)}; then`,
    `  pid=$(cat ${shellQuote(pidFile)} 2>/dev/null || true);`,
    `  if test -n "$pid" && test -r "/proc/$pid/environ" && tr '\\0' '\\n' < "/proc/$pid/environ" | grep -qx ${shellQuote(`ARROBA_REMOTE_HOME_EXTENSION_ROOT=${remoteRoot}`)}; then`,
    '    kill "$pid" 2>/dev/null || true;',
    '  fi;',
    `  rm -f ${shellQuote(pidFile)};`,
    'fi',
  ].join(" ")).catch(() => {})
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
  remoteRoot,
}) {
  await assertRemoteRelayPortFree(options, relayPort)
  const relayPidFile = path.posix.join(remoteRoot, "relay.pid")
  const relay = spawn("ssh", sshArgs(options, remoteEnvCommand({
    ARROBA_REMOTE_REPO: options.hetznerRepo,
    ARROBA_REMOTE_HOME_EXTENSION_ROOT: remoteRoot,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(relayPort),
    ARROBA_RELAY_TOKEN: sharedRelayToken,
    ...(collab ? {
      ARROBA_RELAY_SCOPED_ISSUER: issuer,
      ARROBA_RELAY_SCOPED_HMAC_SECRET: secret,
    } : {}),
  }, `echo $$ > ${shellQuote(relayPidFile)}; exec ./apps/relay/target/debug/arroba-relay`)), { stdio: ["ignore", "ignore", "inherit"] })
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
