import { spawn } from 'node:child_process'
import { constants } from 'node:fs'
import { access, lstat, mkdir, mkdtemp, readFile, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const CHROMIUM_SANDBOX_PREFLIGHT_REASON = 'CHARIOX_CHROMIUM_SANDBOX_PREFLIGHT_FAILED'
const ACTION = 'run as the non-root image user with private writable profile/runtime directories and either enabled user namespaces or a root-owned mode-4755 chrome-sandbox'
const OUTPUT_LIMIT = 4_096

function fail(reason) {
  throw new Error(`${CHROMIUM_SANDBOX_PREFLIGHT_REASON}: ${reason}; ${ACTION}`)
}

export function assertChromiumSandboxPrerequisites(input) {
  if (!Number.isInteger(input.uid) || input.uid <= 0) fail('runtime-user-must-be-non-root')
  for (const [name, directory] of [['profile', input.profile], ['runtime', input.runtime]]) {
    if (!directory?.directory || directory.symbolicLink) fail(`${name}-directory-invalid`)
    if (directory.uid !== input.uid) fail(`${name}-directory-owner-mismatch`)
    if (!directory.writable) fail(`${name}-directory-not-writable`)
    if ((directory.mode & 0o077) !== 0) fail(`${name}-directory-not-private`)
  }
  if (input.platform === 'linux' && !input.setuidSandbox && !input.userNamespaces) {
    fail('linux-sandbox-prerequisite-missing')
  }
}

export function chromiumSandboxProbeArguments(profileDirectory) {
  return [
    '--headless=new',
    '--disable-gpu',
    '--disable-background-networking',
    '--no-first-run',
    '--no-default-browser-check',
    `--user-data-dir=${profileDirectory}`,
    '--dump-dom',
    'about:blank',
  ]
}

async function directoryFacts(directory) {
  const [link, target, writable] = await Promise.all([
    lstat(directory),
    stat(directory),
    access(directory, constants.W_OK | constants.X_OK).then(() => true, () => false),
  ])
  return {
    directory: target.isDirectory(),
    symbolicLink: link.isSymbolicLink(),
    uid: target.uid,
    mode: target.mode & 0o777,
    writable,
  }
}

async function optionalInteger(file) {
  try {
    const value = Number.parseInt((await readFile(file, 'utf8')).trim(), 10)
    return Number.isInteger(value) ? value : null
  } catch {
    return null
  }
}

async function userNamespacesAvailable() {
  const [maximum, unprivileged, namespace] = await Promise.all([
    optionalInteger('/proc/sys/user/max_user_namespaces'),
    optionalInteger('/proc/sys/kernel/unprivileged_userns_clone'),
    stat('/proc/self/ns/user').then(() => true, () => false),
  ])
  return namespace && maximum > 0 && (unprivileged == null || unprivileged === 1)
}

async function resolveExecutable(executable, env) {
  if (executable.includes(path.sep)) return path.resolve(executable)
  for (const directory of (env.PATH ?? '').split(path.delimiter).filter(Boolean)) {
    const candidate = path.join(directory, executable)
    if (await access(candidate, constants.X_OK).then(() => true, () => false)) return candidate
  }
  return null
}

async function setuidSandboxAvailable(executable, env) {
  const resolved = await resolveExecutable(executable, env)
  const candidates = [
    resolved && path.join(path.dirname(resolved), 'chrome-sandbox'),
    '/usr/lib/chromium/chrome-sandbox',
    '/opt/google/chrome/chrome-sandbox',
  ].filter(Boolean)
  for (const candidate of candidates) {
    const metadata = await stat(candidate).catch(() => null)
    if (metadata?.isFile() && metadata.uid === 0 && (metadata.mode & 0o4755) === 0o4755) return true
  }
  return false
}

async function inspectPrerequisites({ executable, profileDirectory, runtimeDirectory, env, platform, uid }) {
  const [profile, runtime, setuidSandbox, userNamespaces] = await Promise.all([
    directoryFacts(profileDirectory).catch(() => null),
    directoryFacts(runtimeDirectory).catch(() => null),
    platform === 'linux' ? setuidSandboxAvailable(executable, env) : false,
    platform === 'linux' ? userNamespacesAvailable() : false,
  ])
  return { platform, uid, profile, runtime, setuidSandbox, userNamespaces }
}

function appendBounded(current, chunk) {
  return `${current}${chunk}`.slice(-OUTPUT_LIMIT)
}

async function runProbe(executable, args, env, timeoutMs) {
  return await new Promise((resolve, reject) => {
    const child = spawn(executable, args, { env, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    let settled = false
    const finish = (callback) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      callback()
    }
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      finish(() => resolve({ code: null, signal: 'SIGKILL', stdout, stderr, timedOut: true }))
    }, timeoutMs)
    child.stdout.on('data', (chunk) => { stdout = appendBounded(stdout, chunk) })
    child.stderr.on('data', (chunk) => { stderr = appendBounded(stderr, chunk) })
    child.once('error', (error) => finish(() => reject(error)))
    child.once('close', (code, signal) => finish(() => resolve({ code, signal, stdout, stderr, timedOut: false })))
  })
}

export async function preflightChromiumSandbox({
  executable,
  profileDirectory,
  runtimeDirectory,
  env = process.env,
  platform = process.platform,
  uid = process.getuid?.(),
  timeoutMs = 15_000,
  probe = runProbe,
  inspect = inspectPrerequisites,
} = {}) {
  if (!executable || !profileDirectory || !runtimeDirectory) fail('configuration-incomplete')
  try {
    await mkdir(profileDirectory, { recursive: true, mode: 0o700 })
    await mkdir(runtimeDirectory, { recursive: true, mode: 0o700 })
  } catch {
    fail('directory-setup-failed')
  }
  assertChromiumSandboxPrerequisites(await inspect({
    executable,
    profileDirectory,
    runtimeDirectory,
    env,
    platform,
    uid,
  }))

  const probeRoot = await mkdtemp(path.join(runtimeDirectory, 'chromium-sandbox-probe-')).catch(() => null)
  if (!probeRoot) fail('probe-profile-unavailable')
  const probeProfile = path.join(probeRoot, 'profile')
  const launchEnv = { ...env, XDG_RUNTIME_DIR: runtimeDirectory }
  try {
    const result = await probe(
      executable,
      chromiumSandboxProbeArguments(probeProfile),
      launchEnv,
      timeoutMs,
    ).catch(() => null)
    if (!result || result.code !== 0 || result.timedOut) fail('sandbox-initialization-failed')
  } finally {
    await rm(probeRoot, { recursive: true, force: true }).catch(() => {})
  }
  return launchEnv
}

async function main() {
  const [, , executable, profileDirectory, runtimeDirectory] = process.argv
  await preflightChromiumSandbox({ executable, profileDirectory, runtimeDirectory })
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    const message = error?.message?.startsWith(`${CHROMIUM_SANDBOX_PREFLIGHT_REASON}:`)
      ? error.message
      : `${CHROMIUM_SANDBOX_PREFLIGHT_REASON}: preflight-error; ${ACTION}`
    process.stderr.write(`${message}\n`)
    process.exitCode = 1
  })
}
