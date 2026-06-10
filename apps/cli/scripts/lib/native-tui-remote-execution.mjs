import { execFile } from "node:child_process"
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
    "  cargo build --manifest-path apps/kernel/Cargo.toml --bin arroba-kernel",
    "  cargo build --manifest-path apps/relay/Cargo.toml --bin arroba-relay",
  ].join("\n")
  await runHetznerCommand(options, [
    `repo=${shellQuote(options.hetznerRepo)}`,
    "missing=",
    'test -d "$repo/.git" || missing="$missing repo"',
    `test -x ${shellQuote(kernelBinary)} || missing="$missing kernel"`,
    `test -x ${shellQuote(relayBinary)} || missing="$missing relay"`,
    `if test -n "$missing"; then printf '%s\n' ${shellQuote(message)} >&2; exit 17; fi`,
  ].join("; "))
}

export async function prepareHetznerWorktree(options, localWorktree) {
  const parent = path.posix.dirname(localWorktree)
  await assertHetznerArrobaBinaries(options)
  await execFileAsync("ssh", sshArgs(options, [
    "set -e",
    `mkdir -p ${shellQuote(parent)}`,
    `git -C ${shellQuote(options.hetznerRepo)} worktree remove --force ${shellQuote(localWorktree)} 2>/dev/null || rm -rf ${shellQuote(localWorktree)}`,
    `git -C ${shellQuote(options.hetznerRepo)} worktree prune`,
    `git -C ${shellQuote(options.hetznerRepo)} worktree add --force --detach ${shellQuote(localWorktree)} HEAD`,
  ].join("; ")))
}

export async function runHetznerCommand(options, command) {
  const { stdout } = await execFileAsync("ssh", sshArgs(options, command), { maxBuffer: 4 * 1024 * 1024 })
  return stdout
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
