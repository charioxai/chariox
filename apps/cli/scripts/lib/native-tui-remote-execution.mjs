import { execFile } from "node:child_process"
import { access, chmod, copyFile, mkdir, readFile, rm } from "node:fs/promises"
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
    `  git clone https://github.com/charioxai/chariox.git ${options.hetznerRepo}`,
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

async function copyFileIfPresent(source, destination, mode) {
  try {
    await copyFile(source, destination)
  } catch (error) {
    if (error?.code === "ENOENT") return false
    throw error
  }
  await chmod(destination, mode)
  return true
}

async function filesArePresent(paths) {
  const present = await Promise.all(paths.map(async (candidate) => {
    try {
      await access(candidate)
      return true
    } catch (error) {
      if (error?.code === "ENOENT") return false
      throw error
    }
  }))
  return present.every(Boolean)
}

export async function seedLocalOpenCodeRuntimeProfile({
  sourceDataHome,
  sourceCacheHome,
  destinationXdgDataHome,
  destinationXdgCacheHome,
}) {
  const destinationDataHome = path.join(destinationXdgDataHome, "opencode")
  const destinationCacheHome = path.join(destinationXdgCacheHome, "opencode")
  await mkdir(destinationDataHome, { recursive: true, mode: 0o700 })
  await mkdir(destinationCacheHome, { recursive: true, mode: 0o700 })

  const credentialPath = path.join(destinationDataHome, "auth.json")
  const destinationCatalogPaths = ["models.json", "version"]
    .map((filename) => path.join(destinationCacheHome, filename))
  try {
    const credentialCopied = await copyFileIfPresent(
      path.join(sourceDataHome, "auth.json"),
      credentialPath,
      0o600,
    )
    const sourceCatalogPaths = ["models.json", "version"]
      .map((filename) => path.join(sourceCacheHome, filename))
    if (await filesArePresent(sourceCatalogPaths)) {
      for (let index = 0; index < sourceCatalogPaths.length; index += 1) {
        await copyFile(sourceCatalogPaths[index], destinationCatalogPaths[index])
        await chmod(destinationCatalogPaths[index], 0o600)
      }
    }
    return credentialCopied ? credentialPath : null
  } catch (error) {
    await Promise.all([
      rm(credentialPath, { force: true }),
      ...destinationCatalogPaths.map((candidate) => rm(candidate, { force: true })),
    ])
    throw error
  }
}

export function hetznerOpenCodeRuntimeProfileSeedCommand(runtimeRoot) {
  if (!/^\/tmp\/arb-remote-native-tui-\d+-\d+$/.test(runtimeRoot)) {
    throw new Error(`refusing unexpected Hetzner native TUI runtime root: ${runtimeRoot}`)
  }
  const destinationDataHome = path.posix.join(runtimeRoot, "xdg-data", "opencode")
  const destinationCacheHome = path.posix.join(runtimeRoot, "xdg-cache", "opencode")
  const sourceDataHome = "/root/.local/share/opencode"
  const sourceCacheHome = "/root/.cache/opencode"
  return [
    "set -e",
    `install -d -m 700 ${shellQuote(destinationDataHome)} ${shellQuote(destinationCacheHome)}`,
    `if test -s ${shellQuote(path.posix.join(sourceDataHome, "auth.json"))}; then install -m 600 ${shellQuote(path.posix.join(sourceDataHome, "auth.json"))} ${shellQuote(path.posix.join(destinationDataHome, "auth.json"))}; fi`,
    `if test -s ${shellQuote(path.posix.join(sourceCacheHome, "models.json"))} && test -s ${shellQuote(path.posix.join(sourceCacheHome, "version"))}; then install -m 600 ${shellQuote(path.posix.join(sourceCacheHome, "models.json"))} ${shellQuote(path.posix.join(destinationCacheHome, "models.json"))}; install -m 600 ${shellQuote(path.posix.join(sourceCacheHome, "version"))} ${shellQuote(path.posix.join(destinationCacheHome, "version"))}; fi`,
  ].join("; ")
}

export async function seedHetznerOpenCodeRuntimeProfile(options, runtimeRoot) {
  await runHetznerCommand(options, hetznerOpenCodeRuntimeProfileSeedCommand(runtimeRoot))
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

export async function stopHetznerRuntimeBeforeClaudeTrustRestore({
  stopWorker,
  stopRelay,
  restoreTrust,
}) {
  await stopWorker()
  await stopRelay()
  if (restoreTrust) await restoreTrust()
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

export async function copyLocalPathToHetzner(
  options,
  localPath,
  remotePath,
  { recursive = false } = {},
) {
  await runHetznerCommand(options, `mkdir -p ${shellQuote(path.posix.dirname(remotePath))}`)
  await execFileAsync("scp", [
    "-i",
    options.hetznerKey,
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    ...(recursive ? ["-r"] : []),
    localPath,
    `${options.hetznerHost}:${remotePath}`,
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

export function applyClaudeWorkspaceTrust(configInput, workspace) {
  const config = configInput && typeof configInput === "object"
    ? structuredClone(configInput)
    : {}
  const projects = config.projects && typeof config.projects === "object"
    ? { ...config.projects }
    : {}
  const hadEntry = Object.prototype.hasOwnProperty.call(projects, workspace)
  const entry = hadEntry ? structuredClone(projects[workspace]) : null
  const existing = entry && typeof entry === "object" ? entry : null
  const template = existing ?? Object.values(projects).find((value) => (
    value && typeof value === "object" && Object.prototype.hasOwnProperty.call(value, "hasTrustDialogAccepted")
  )) ?? {}
  projects[workspace] = {
    ...template,
    allowedTools: Array.isArray(template.allowedTools) ? template.allowedTools : [],
    hasTrustDialogAccepted: true,
    projectOnboardingSeenCount: Math.max(Number(template.projectOnboardingSeenCount) || 0, 1),
  }
  config.projects = projects
  return { config, state: { workspace, hadEntry, entry } }
}

export function restoreClaudeWorkspaceTrust(configInput, state) {
  const config = configInput && typeof configInput === "object"
    ? structuredClone(configInput)
    : {}
  const projects = config.projects && typeof config.projects === "object"
    ? { ...config.projects }
    : {}
  if (state.hadEntry) projects[state.workspace] = structuredClone(state.entry)
  else delete projects[state.workspace]
  config.projects = projects
  return config
}

export function prepareClaudeWorkspaceTrustConfigText(configText, workspace) {
  const originalConfigExisted = configText !== null
  const config = originalConfigExisted ? JSON.parse(configText) : {}
  const prepared = applyClaudeWorkspaceTrust(config, workspace)
  return {
    config: prepared.config,
    state: {
      ...prepared.state,
      originalConfigExisted,
      originalConfigBase64: originalConfigExisted
        ? Buffer.from(configText, "utf8").toString("base64")
        : null,
    },
  }
}

export function restoreClaudeConfigText(state) {
  if (typeof state?.originalConfigExisted !== "boolean") {
    throw new Error("Claude workspace trust state is missing the original config status")
  }
  if (!state.originalConfigExisted) return null
  if (typeof state.originalConfigBase64 !== "string") {
    throw new Error("Claude workspace trust state is missing the original config bytes")
  }
  return Buffer.from(state.originalConfigBase64, "base64").toString("utf8")
}

export async function prepareHetznerClaudeWorkspaceTrust(options, workspace, statePath) {
  validateClaudeWorkspaceTrustPaths(workspace, statePath)
  const script = `
const fs = require("fs")
const path = require("path")
const applyClaudeWorkspaceTrust = ${applyClaudeWorkspaceTrust.toString()}
const prepareClaudeWorkspaceTrustConfigText = ${prepareClaudeWorkspaceTrustConfigText.toString()}
const configPath = "/root/.claude.json"
const workspace = ${JSON.stringify(workspace)}
const statePath = ${JSON.stringify(statePath)}
if (fs.existsSync(statePath)) throw new Error("Claude workspace trust restoration state already exists")
const configText = fs.existsSync(configPath) ? fs.readFileSync(configPath, "utf8") : null
const prepared = prepareClaudeWorkspaceTrustConfigText(configText, workspace)
fs.mkdirSync(path.dirname(statePath), { recursive: true })
fs.writeFileSync(statePath, JSON.stringify(prepared.state), { mode: 0o600 })
const tempPath = configPath + ".arroba-" + process.pid + "-" + Date.now()
fs.writeFileSync(tempPath, JSON.stringify(prepared.config, null, 2), { mode: 0o600 })
fs.renameSync(tempPath, configPath)
fs.chmodSync(configPath, 0o600)
`
  await runHetznerCommand(options, `node -e ${shellQuote(script)}`)
}

export async function restoreHetznerClaudeWorkspaceTrust(options, workspace, statePath) {
  validateClaudeWorkspaceTrustPaths(workspace, statePath)
  const script = `
const fs = require("fs")
const restoreClaudeWorkspaceTrust = ${restoreClaudeWorkspaceTrust.toString()}
const restoreClaudeConfigText = ${restoreClaudeConfigText.toString()}
const configPath = "/root/.claude.json"
const workspace = ${JSON.stringify(workspace)}
const statePath = ${JSON.stringify(statePath)}
if (fs.existsSync(statePath)) {
  const state = JSON.parse(fs.readFileSync(statePath, "utf8"))
  if (!state || state.workspace !== workspace || typeof state.hadEntry !== "boolean") {
    throw new Error("Claude workspace trust restoration state does not match this drill")
  }
  if (Object.prototype.hasOwnProperty.call(state, "originalConfigExisted")) {
    const originalConfigText = restoreClaudeConfigText(state)
    if (originalConfigText === null) {
      fs.rmSync(configPath, { force: true })
    } else {
      const tempPath = configPath + ".arroba-" + process.pid + "-" + Date.now()
      fs.writeFileSync(tempPath, originalConfigText, { mode: 0o600 })
      fs.renameSync(tempPath, configPath)
      fs.chmodSync(configPath, 0o600)
      if (fs.readFileSync(configPath, "utf8") !== originalConfigText) {
        throw new Error("Claude config bytes did not restore exactly")
      }
    }
  } else {
    const config = fs.existsSync(configPath) ? JSON.parse(fs.readFileSync(configPath, "utf8")) : {}
    const restored = restoreClaudeWorkspaceTrust(config, state)
    const tempPath = configPath + ".arroba-" + process.pid + "-" + Date.now()
    fs.writeFileSync(tempPath, JSON.stringify(restored, null, 2), { mode: 0o600 })
    fs.renameSync(tempPath, configPath)
    fs.chmodSync(configPath, 0o600)
  }
  fs.unlinkSync(statePath)
}
`
  await runHetznerCommand(options, `node -e ${shellQuote(script)}`)
}

function validateClaudeWorkspaceTrustPaths(workspace, statePath) {
  if (!path.posix.isAbsolute(workspace)) {
    throw new Error(`Claude workspace trust path must be absolute: ${workspace}`)
  }
  if (!/^\/tmp\/arb-remote-native-tui-\d+-\d+\/claude-workspace-trust\.json$/.test(statePath)) {
    throw new Error(`refusing unexpected Claude workspace trust state path: ${statePath}`)
  }
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

export function hetznerNativeRuntimeTempDir(runtimeRoot) {
  if (!/^\/tmp\/arb-remote-native-tui-\d+-\d+$/.test(runtimeRoot)) {
    throw new Error(`refusing unexpected Hetzner native TUI runtime root: ${runtimeRoot}`)
  }
  return path.posix.join(runtimeRoot, "tmp")
}

export async function removeHetznerNativeRuntimePaths(options, paths) {
  const command = hetznerNativeRuntimeCleanupCommand(paths)
  if (command) await runHetznerCommand(options, command).catch(() => {})
}

export async function ensureExecutionDirectory(options, remoteExecution, dirPath) {
  if (remoteExecution) {
    await runHetznerCommand(options, `mkdir -p ${shellQuote(dirPath)}`)
  } else {
    await mkdir(dirPath, { recursive: true })
  }
}

export async function removeExecutionFile(options, remoteExecution, filePath) {
  if (remoteExecution) {
    await runHetznerCommand(options, `rm -f ${shellQuote(filePath)}`).catch(() => {})
  } else {
    await rm(filePath, { force: true })
  }
}

export async function waitForExecutionFileContent(options, remoteExecution, filePath, expected, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  let last = ""
  while (Date.now() < deadline) {
    const content = remoteExecution
      ? await runHetznerCommand(options, `cat ${shellQuote(filePath)}`).catch(() => "")
      : await readFile(filePath, "utf8").catch(() => "")
    last = content
    if (content.includes(expected)) return content
    await sleep(1_000)
  }
  const location = remoteExecution ? "remote" : "local"
  throw new Error(`timed out waiting for ${location} file ${filePath} to contain ${expected}; last=${last}`)
}

export function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`
}
