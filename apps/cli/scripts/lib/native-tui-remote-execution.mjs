import { execFile } from "node:child_process"
import { mkdir } from "node:fs/promises"
import path from "node:path"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"

const execFileAsync = promisify(execFile)

export function sshArgs(options, remoteCommand) {
  return [
    "-i",
    options.hetznerKey,
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    options.hetznerHost,
    remoteCommand,
  ]
}

export function remoteEnvCommand(env, command) {
  const assignments = Object.entries(env)
    .map(([key, value]) => `${key}=${shellQuote(String(value))}`)
    .join(" ")
  return `cd ${shellQuote(env.ARROBA_REMOTE_REPO)} && env ${assignments} bash -lc ${shellQuote(command)}`
}

export async function assertHetznerArrobaBinaries(options) {
  const kernelBinary = path.posix.join(options.hetznerRepo, "apps/kernel/target/debug/arroba-kernel")
  const relayBinary = path.posix.join(options.hetznerRepo, "apps/relay/target/debug/arroba-relay")
  const message = [
    `Hetzner Arroba checkout is not ready at ${options.hetznerRepo}.`,
    `Expected executable binaries: ${kernelBinary} and ${relayBinary}.`,
    "Prepare the worker with:",
    `  git clone https://github.com/mgutierrez09/arroba.git ${options.hetznerRepo}`,
    `  cd ${options.hetznerRepo}`,
    "  git checkout main && git reset --hard origin/main",
    "  export PATH=/root/.cargo/bin:$PATH",
    "  rustup toolchain install stable --profile minimal",
    "  CARGO_TARGET_DIR=apps/kernel/target cargo build --manifest-path apps/kernel/Cargo.toml --bin arroba-kernel",
    "  CARGO_TARGET_DIR=apps/relay/target cargo build --manifest-path apps/relay/Cargo.toml --bin arroba-relay",
  ].join("\n")
  await runHetznerCommand(options, [
    `repo=${shellQuote(options.hetznerRepo)}`,
    "missing=",
    'test -e "$repo/.git" || missing="$missing repo"',
    `test -x ${shellQuote(kernelBinary)} || missing="$missing kernel"`,
    `test -x ${shellQuote(relayBinary)} || missing="$missing relay"`,
    `if test -n "$missing"; then printf '%s\n' ${shellQuote(message)} >&2; exit 17; fi`,
  ].join("; "))
}

export async function prepareHetznerWorktree(options, localWorktree) {
  const parent = path.posix.dirname(localWorktree)
  await assertHetznerArrobaBinaries(options)
  const [{ stdout: localCommit }, remoteCommit] = await Promise.all([
    execFileAsync("git", ["-C", localWorktree, "rev-parse", "HEAD"]),
    runHetznerCommand(options, `git -C ${shellQuote(options.hetznerRepo)} rev-parse HEAD`),
  ])
  assertMatchingHetznerCheckoutCommit({
    localCommit,
    remoteCommit,
    remoteRepo: options.hetznerRepo,
  })
  await assertHetznerArrobaBinaryFreshness(options)
  await execFileAsync("ssh", sshArgs(options, [
    "set -e",
    `mkdir -p ${shellQuote(parent)}`,
    `git -C ${shellQuote(options.hetznerRepo)} worktree remove --force ${shellQuote(localWorktree)} 2>/dev/null || rm -rf ${shellQuote(localWorktree)}`,
    `git -C ${shellQuote(options.hetznerRepo)} worktree prune`,
    `git -C ${shellQuote(options.hetznerRepo)} worktree add --force --detach ${shellQuote(localWorktree)} HEAD`,
  ].join("; ")))
}

export async function assertHetznerArrobaBinaryFreshness(options) {
  const repo = options.hetznerRepo
  const kernelBinary = path.posix.join(repo, "apps/kernel/target/debug/arroba-kernel")
  const relayBinary = path.posix.join(repo, "apps/relay/target/debug/arroba-relay")
  const command = [
    "set -euo pipefail",
    `repo=${shellQuote(repo)}`,
    `kernel=${shellQuote(kernelBinary)}`,
    `relay=${shellQuote(relayBinary)}`,
    "newer_source() {",
    "  local binary=$1; shift;",
    "  git -C \"$repo\" ls-files -- \"$@\" | while IFS= read -r relative; do",
    "    if test \"$repo/$relative\" -nt \"$binary\"; then printf '%s' \"$relative\"; break; fi;",
    "  done;",
    "}",
    "printf 'kernel_newer=%s\\n' \"$(newer_source \"$kernel\" Cargo.toml Cargo.lock apps/kernel apps/relay)\"",
    "printf 'relay_newer=%s\\n' \"$(newer_source \"$relay\" Cargo.toml Cargo.lock apps/relay)\"",
  ].join("\n")
  const output = await runHetznerCommand(options, `bash -lc ${shellQuote(command)}`)
  const info = Object.fromEntries(output
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => {
      const separator = line.indexOf("=")
      return [line.slice(0, separator), line.slice(separator + 1)]
    }))
  assertHetznerBinaryFreshness({
    remoteRepo: repo,
    kernelNewerPath: info.kernel_newer ?? "",
    relayNewerPath: info.relay_newer ?? "",
  })
}

export function assertHetznerBinaryFreshness({ remoteRepo, kernelNewerPath, relayNewerPath }) {
  const stale = [
    kernelNewerPath ? `kernel binary is older than ${kernelNewerPath}` : null,
    relayNewerPath ? `relay binary is older than ${relayNewerPath}` : null,
  ].filter(Boolean)
  if (stale.length === 0) return
  throw new Error([
    `Hetzner Arroba binaries are stale at ${remoteRepo}: ${stale.join("; ")}.`,
    "Rebuild into apps/kernel/target and apps/relay/target before running the drill.",
  ].join("\n"))
}

export function assertMatchingHetznerCheckoutCommit({ localCommit, remoteCommit, remoteRepo }) {
  const local = String(localCommit ?? "").trim()
  const remote = String(remoteCommit ?? "").trim()
  if (!/^[0-9a-f]{40}$/i.test(local) || !/^[0-9a-f]{40}$/i.test(remote)) {
    throw new Error(`could not verify local and remote checkout commits for ${remoteRepo}`)
  }
  if (local !== remote) {
    throw new Error(
      `remote worker checkout \`${remoteRepo}\` is at commit ${remote}, but home checkout expects ${local}; prepare the remote checkout at the home commit and rebuild its binaries`,
    )
  }
}

export async function runHetznerCommand(options, command) {
  const { stdout } = await execFileAsync("ssh", sshArgs(options, command), { maxBuffer: 4 * 1024 * 1024 })
  return stdout
}

export async function assertHetznerTcpPortAvailable(options, port, label = "Hetzner TCP port") {
  const pids = (await runHetznerCommand(
    options,
    `if command -v lsof >/dev/null 2>&1; then lsof -tiTCP:${Number(port)} -sTCP:LISTEN 2>/dev/null || true; fi`,
  )).trim()
  if (pids) {
    throw new Error(`${label} ${port} is already in use by pid(s) ${pids}; choose another run or wait for the owner to finish`)
  }
}

export async function stopHetznerProcessByEnv(options, expectedEnv) {
  const checks = Object.entries(expectedEnv).map(([key, value]) => (
    `  printf '%s\\n' "$env_text" | grep -qx ${shellQuote(`${key}=${value}`)} || continue;`
  ))
  await runHetznerCommand(options, [
    'for env_file in /proc/[0-9]*/environ; do',
    '  test -r "$env_file" || continue;',
    '  pid=${env_file#/proc/}; pid=${pid%/environ};',
    '  env_text=$(tr "\\0" "\\n" < "$env_file" 2>/dev/null || true);',
    ...checks,
    '  kill "$pid" 2>/dev/null || true;',
    '  for _ in 1 2 3 4 5; do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done;',
    '  kill -9 "$pid" 2>/dev/null || true;',
    'done',
  ].join(' ')).catch(() => {})
}

export async function copyHetznerDirectoryToLocal(options, remoteDir, localDir) {
  await mkdir(localDir, { recursive: true })
  await execFileAsync("scp", [
    "-i",
    options.hetznerKey,
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-r",
    `${options.hetznerHost}:${remoteDir}/.`,
    `${localDir}/`,
  ], { maxBuffer: 4 * 1024 * 1024 })
}

export function hetznerWorktreeCleanupCommand(remoteRepo, remoteWorktree) {
  return [
    `git -C ${shellQuote(remoteRepo)} worktree remove --force ${shellQuote(remoteWorktree)} 2>/dev/null || rm -rf -- ${shellQuote(remoteWorktree)}`,
    `git -C ${shellQuote(remoteRepo)} worktree prune`,
  ].join("; ")
}

export async function removeHetznerWorktree(options, remoteWorktree) {
  await runHetznerCommand(
    options,
    hetznerWorktreeCleanupCommand(options.hetznerRepo, remoteWorktree),
  ).catch(() => {})
}

export function hetznerNativeRuntimeCleanupCommand(paths) {
  const uniquePaths = [...new Set(paths.filter(Boolean))]
  for (const runtimePath of uniquePaths) {
    if (!/^\/tmp\/arb-remote-native-tui-\d+(?:-\d+)?$/.test(runtimePath)) {
      throw new Error(`refusing to remove unexpected Hetzner native TUI runtime path: ${runtimePath}`)
    }
  }
  return uniquePaths.length > 0
    ? `rm -rf -- ${uniquePaths.map(shellQuote).join(" ")}`
    : null
}

export async function removeHetznerNativeRuntimePaths(options, paths) {
  const command = hetznerNativeRuntimeCleanupCommand(paths)
  if (command) await runHetznerCommand(options, command).catch(() => {})
}

export async function ensureExecutionDirectory(options, remoteExecution, dirPath) {
  if (remoteExecution) {
    await runHetznerCommand(options, `mkdir -p ${shellQuote(dirPath)}`)
  }
}

export async function removeExecutionFile(options, remoteExecution, filePath) {
  if (remoteExecution) {
    await runHetznerCommand(options, `rm -f ${shellQuote(filePath)}`).catch(() => {})
  }
}

export async function waitForExecutionFileContent(options, remoteExecution, filePath, expected, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  let last = ""
  while (Date.now() < deadline) {
    const content = await runHetznerCommand(options, `cat ${shellQuote(filePath)}`).catch(() => "")
    last = content
    if (content.includes(expected)) return content
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for remote file ${filePath} to contain ${expected}; last=${last}`)
}

export function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`
}
