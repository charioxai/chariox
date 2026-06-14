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

export const DEFAULT_REMOTE_HOME_EXTENSION_HETZNER_MIN_FREE_KB = 262144

export async function ensureRemoteHomeExtensionHetznerWorkspace(options, {
  remoteRoot,
  workerWorktree,
  expectedRepoHead = null,
  minFreeKb = DEFAULT_REMOTE_HOME_EXTENSION_HETZNER_MIN_FREE_KB,
}) {
  await assertHetznerArrobaBinaries(options)
  await runHetznerCommand(options, remoteHomeExtensionHetznerPreflightCommand({
    hetznerRepo: options.hetznerRepo,
    remoteRoot,
    workerWorktree,
    expectedRepoHead,
    minFreeKb,
  }))
}

export function remoteHomeExtensionHetznerPreflightCommand({
  hetznerRepo,
  remoteRoot,
  workerWorktree,
  expectedRepoHead = null,
  minFreeKb = DEFAULT_REMOTE_HOME_EXTENSION_HETZNER_MIN_FREE_KB,
}) {
  const expected = expectedRepoHead == null ? "" : String(expectedRepoHead).trim()
  return [
    "set -e",
    `repo=${shellQuote(hetznerRepo)}`,
    `remote_root=${shellQuote(remoteRoot)}`,
    `worker_worktree=${shellQuote(workerWorktree)}`,
    `expected_head=${shellQuote(expected)}`,
    `min_free_kb=${Number(minFreeKb)}`,
    "actual_head=$(git -C \"$repo\" rev-parse HEAD 2>/dev/null || true)",
    "if test -n \"$expected_head\" && test \"$actual_head\" != \"$expected_head\"; then",
    "  printf 'remote worker checkout `%s` is at commit %s, but home checkout expects %s. Upgrade/rebuild the remote worker checkout and restart the worker kernel, then rerun the drill.\\n' \"$repo\" \"${actual_head:-unknown}\" \"$expected_head\" >&2",
    "  exit 18",
    "fi",
    "check_free_kb() {",
    "  label=\"$1\"",
    "  target=\"$2\"",
    "  existing=\"$target\"",
    "  while ! test -e \"$existing\"; do",
    "    parent=$(dirname \"$existing\")",
    "    if test \"$parent\" = \"$existing\"; then break; fi",
    "    existing=\"$parent\"",
    "  done",
    "  free_kb=$(df -Pk \"$existing\" | awk 'NR == 2 { print $4 }')",
    "  if test -z \"$free_kb\" || test \"$free_kb\" -lt \"$min_free_kb\"; then",
    "    printf 'remote host filesystem for %s at %s has %sKB free; remote worker drills need at least %sKB. Free disk on the remote host or choose a clean worker checkout/artifact root, then rerun the drill.\\n' \"$label\" \"$existing\" \"${free_kb:-unknown}\" \"$min_free_kb\" >&2",
    "    exit 19",
    "  fi",
    "}",
    "check_free_kb repo \"$repo\"",
    "check_free_kb remote-root \"$remote_root\"",
    "check_free_kb worker-worktree \"$worker_worktree\"",
    "mkdir -p \"$remote_root\" \"$worker_worktree\"",
  ].join("\n")
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

export async function stopRemoteHomeExtensionHetznerWorker(options, {
  remoteRoot,
  workerDaemonId,
}) {
  const pidFile = path.posix.join(remoteRoot, "worker.pid")
  await runHetznerCommand(options, [
    "stop_owned_pid() {",
    "  pid=\"$1\";",
    "  test -n \"$pid\" || return 0;",
    "  test -r \"/proc/$pid/environ\" || return 0;",
    `  env_text=$(tr '\\0' '\\n' < "/proc/$pid/environ" 2>/dev/null || true);`,
    `  printf '%s\\n' "$env_text" | grep -qx ${shellQuote(`ARROBA_DAEMON_ID=${workerDaemonId}`)} || return 0;`,
    `  printf '%s\\n' "$env_text" | grep -qx ${shellQuote(`ARROBA_REMOTE_HOME_EXTENSION_ROOT=${remoteRoot}`)} || return 0;`,
    "  kill \"$pid\" 2>/dev/null || true;",
    "  for _ in 1 2 3 4 5; do kill -0 \"$pid\" 2>/dev/null || return 0; sleep 0.2; done;",
    "  kill -9 \"$pid\" 2>/dev/null || true;",
    "};",
    `if test -f ${shellQuote(pidFile)}; then`,
    `  stop_owned_pid "$(cat ${shellQuote(pidFile)} 2>/dev/null || true)";`,
    `  rm -f ${shellQuote(pidFile)};`,
    "fi;",
    "for env_file in /proc/[0-9]*/environ; do",
    "  test -r \"$env_file\" || continue;",
    "  pid=${env_file#/proc/}; pid=${pid%/environ};",
    "  stop_owned_pid \"$pid\";",
    "done",
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
  const workerPidFile = path.posix.join(remoteRoot, "worker.pid")
  return spawn("ssh", sshArgs(options, remoteEnvCommand({
    ARROBA_REMOTE_REPO: options.hetznerRepo,
    ARROBA_REMOTE_HOME_EXTENSION_ROOT: remoteRoot,
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
  }, `mkdir -p ${shellQuote(remoteRoot)} ${shellQuote(workerWorktree)} && echo $$ > ${shellQuote(workerPidFile)} && exec ./apps/kernel/target/debug/arroba-kernel`)), {
    stdio: ["ignore", "ignore", "inherit"],
  })
}
