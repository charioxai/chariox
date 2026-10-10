// MP-07 / MP-08 / MP-11: self-setup uses the SSH installer's verified lifecycle.
import { createHash, verify } from "node:crypto"
import { spawn } from "node:child_process"
import { copyFile, lstat, mkdir, mkdtemp, readFile, readlink, realpath, rename, rm, symlink, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { runMachine, command, regular, tree, validateRequest } from "../kernel/ssh-machine/remote.mjs"
import { launchdDefinition, launchdManager } from "./launchd.mjs"
import { publicKeyFromHex, verifyBundle } from "../../scripts/release-bundle.mjs"

const fail = message => { throw new Error(message) }
export function targetPlatform(os = process.platform, arch = process.arch) {
  const value = `${os}-${arch}`
  if (!["linux-x64", "darwin-arm64"].includes(value)) fail("supported Setup targets are Linux x86_64 and macOS arm64")
  return value
}
export function publicUrl(value, allowPublicAssetQuery = false) {
  const url = new URL(value)
  if (url.username || url.password || (url.search && !allowPublicAssetQuery) || url.hash || !(url.protocol === "https:" || (url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)))) fail("Setup requires HTTPS release/Cloud URLs (loopback allowed for tests)")
  return value.replace(/\/$/, "")
}
async function download(url, path, limit) {
  const initial = new URL(publicUrl(url))
  let response
  for (let redirects = 0; ; redirects++) {
    response = await fetch(publicUrl(url, true), { redirect: "manual", signal: AbortSignal.timeout(300_000) })
    if (![301, 302, 303, 307, 308].includes(response.status)) break
    if (redirects >= 5 || !response.headers.get("location")) fail("release redirect refused")
    const next = new URL(response.headers.get("location"), url).href
    publicUrl(next, true)
    const target = new URL(next)
    if (initial.protocol === "https:" ? target.protocol !== "https:" : target.origin !== initial.origin) fail("release redirect transport refused")
    await response.body?.cancel()
    url = next
  }
  if (!response.ok || !response.body) fail("release download failed")
  if (Number(response.headers.get("content-length")) > limit) fail("release download exceeded limit")
  const { open } = await import("node:fs/promises")
  const file = await open(path, "wx", 0o600)
  let count = 0
  try {
    for await (const chunk of response.body) { count += chunk.length; if (count > limit) fail("release download exceeded limit"); await file.write(chunk) }
  } finally { await file.close() }
}
const sha = bytes => createHash("sha256").update(bytes).digest("hex")
export function bundleAdapter({ version, platform, extractorSource }) {
  return {
    kernelPath: "bin/chariox-kernel", manifestPath: "manifest.json",
    async extract(image, stage) {
      if (!extractorSource) fail("bounded release extractor missing")
      const helper = join(stage, "extract-release.py")
      await writeFile(helper, extractorSource, { mode: 0o600 })
      const unpacked = join(stage, "unpacked")
      await command("python3", [helper, join(stage, "release.tar.gz"), unpacked])
      // Public release archives carry exactly one versioned top-level directory.
      const { readdir } = await import("node:fs/promises")
      const name = `chariox-${version}-${platform}`
      if (JSON.stringify(await readdir(unpacked)) !== JSON.stringify([name])) fail("unexpected release archive layout")
      await rename(join(unpacked, name), image)
    },
    async verify(image, digest, pins) {
      const key = (await regular(join(pins, "release-public-pin"), 128)).toString().trim()
      const receipt = await verifyBundle(image, key)
      if (`sha256:${receipt.manifestSha256}` !== digest || receipt.platform !== platform) fail("release identity/platform does not match selection")
      // Execute permissions come from the verified inventory, not unsigned tar modes.
      const manifest = JSON.parse(await readFile(join(image, "manifest.json")))
      for (const name of ["chariox", "chariox-kernel", "chariox-setup"]) if (!manifest.files.some(file => file.path === `bin/${name}`)) fail("chosen release does not carry CLI, kernel and Setup together")
      const { chmod } = await import("node:fs/promises")
      for (const entry of manifest.files) {
        if (!/^(0[4567][0-7]{2})$/.test(entry.mode)) fail("invalid signed release mode")
        await chmod(join(image, entry.path), Number.parseInt(entry.mode, 8) & 0o755)
      }
    },
  }
}
export async function prepareRelease({ stage, version, platform, releaseBase, publicKeyHex }) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version ?? "")) fail("a pinned release version is required")
  publicKeyFromHex(publicKeyHex)
  const base = `${publicUrl(releaseBase)}/v${version}/chariox-${version}-${platform}`
  await download(`${base}.manifest.json`, join(stage, "manifest.json"), 16 << 20)
  await download(`${base}.manifest.sig`, join(stage, "manifest.sig"), 128)
  const bytes = await readFile(join(stage, "manifest.json")), sig = await readFile(join(stage, "manifest.sig"), "utf8")
  if (!/^[a-f0-9]{128}$/.test(sig) || !verify(null, bytes, publicKeyFromHex(publicKeyHex), Buffer.from(sig, "hex"))) fail("release signature refused")
  const manifest = JSON.parse(bytes)
  if (manifest.schema !== "chariox.release-bundle.v1" || manifest.version !== version || manifest.platform !== platform) fail("release identity/platform refused")
  await writeFile(join(stage, "release-public-pin"), publicKeyHex, { mode: 0o600 })
  // The distribution bundle's independently pinned key also signs the runtime key inventory.
  await writeFile(join(stage, "builder-public-pin"), publicKeyHex, { mode: 0o600 })
  await download(`${base}.tar.gz`, join(stage, "release.tar.gz"), 2 * 1024 ** 3)
  return `sha256:${sha(bytes)}`
}
// MP-11: own child only; bounded stdout, no stderr or credential forwarding.
export async function deviceEnroll(binary, input, env, openBrowser, notice) {
  const child = spawn(binary, ["--owner-managed-device-enroll-stdin"], { env, stdio: ["pipe", "pipe", "ignore"] })
  const stop = () => {
    if (child.exitCode !== null || child.signalCode !== null) return
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) fail("refusing unsafe Setup process signal")
    child.kill("SIGKILL")
  }
  child.stdin.on("error", () => {})
  child.stdin.end(JSON.stringify(input))
  let buffer = "", identity, failure, total = 0
  let pending = Promise.resolve()
  child.stdout.on("data", chunk => {
    total += chunk.length
    if (total > 16384) { failure = new Error("bounded enrollment output exceeded"); stop(); return }
    buffer += chunk.toString("utf8")
    let index
    while ((index = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, index); buffer = buffer.slice(index + 1)
      pending = pending.then(async () => {
        const value = JSON.parse(line)
        if (value.verificationUrl) {
          const url = new URL(value.verificationUrl), api = new URL(input.apiUrl)
          if (url.origin !== api.origin || url.username || url.password || typeof value.userCode !== "string") fail("invalid device approval URL")
          notice(`Approve this machine: ${value.verificationUrl}\nCode: ${value.userCode}`)
          await openBrowser(value.verificationUrl)
        } else if (value.kernelId && value.machineId && value.userId && value.publicKeyThumbprint) identity = value
        else fail("invalid enrollment output")
      }).catch(() => { failure = new Error("device enrollment failed"); stop() })
    }
  })
  const timer = setTimeout(() => { failure = new Error("device enrollment timed out"); stop() }, 660_000)
  try {
    await new Promise((yes, no) => { child.once("error", () => no(new Error("kernel could not start enrollment"))); child.once("close", code => code === 0 ? yes() : no(new Error("kernel enrollment refused"))) })
    await pending
    if (failure || !identity || buffer.trim()) throw failure ?? new Error("kernel enrollment incomplete")
    return identity
  } finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) stop() }
}
function kernelEnv(home, id, port) {
  const env = { ...process.env, HOME: home }
  for (const name of Object.keys(env)) if (name.startsWith("CHARIOX_")) delete env[name]
  return { ...env, CHARIOX_HOME: `${home}/.chariox/dev/ssh-machines/${id}`, CHARIOX_KERNEL_HOST: "127.0.0.1", CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_HOST: "127.0.0.1", CHARIOX_MCP_PORT: String(port + 1) }
}
// One exclusive link: refuse existing unowned CLI executables rather than replacing them.
function cliNames(id) { return ["chariox", "chariox-setup"].map(name => ({ name, link: id === "local" ? name : `${name}-${id}` })) }
async function cliLink(home, root, id, remove = false) {
  const bin = await tree(home, ".local/bin")
  for (const { name, link } of cliNames(id)) {
    const path = join(bin, link), target = `${root}/current/bin/${name}`
    const old = await lstat(path).catch(e => e.code === "ENOENT" ? null : Promise.reject(e))
    if (old && (!old.isSymbolicLink() || await readlink(path) !== target)) fail("CLI path belongs to another install")
    if (remove) { if (old) await rm(path) }
    else if (!old) await symlink(target, path)
  }
}
export async function installLocal(options) {
  const { home = process.env.HOME, version, publicKeyHex, releaseBase = "https://github.com/charioxai/chariox/releases/download", apiUrl = "https://chariox.com", installId = "local", port = 55139, action = "install", ticket, userId, extractorSource, openBrowser = async () => {}, notice = message => process.stdout.write(`${message}\n`), serviceManager, kernelCommand } = options
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version ?? "")) fail("a pinned release version is required")
  const platform = targetPlatform(options.platform?.split("-")[0], options.platform?.split("-")[1])

  publicUrl(apiUrl)
  validateRequest({ action, installId, port, releaseDigest: `sha256:${"a".repeat(64)}` })
  if (!home || !home.startsWith("/") || home === "/" || await realpath(home) !== home) fail("user HOME must be canonical and absolute")
  const parent = await tree(home, ".local/share/chariox/ssh-machines")
  const root = join(parent, installId)
  const stage = await mkdtemp(join(parent, ".setup-"))
  try {
    // Preflight CLI ownership before changing any install/service state.
    const bin = await tree(home, ".local/bin")
    for (const { name, link } of cliNames(installId)) {
      const path = join(bin, link), old = await lstat(path).catch(e => e.code === "ENOENT" ? null : Promise.reject(e))
      if (old && (!old.isSymbolicLink() || await readlink(path) !== `${root}/current/bin/${name}`)) fail("CLI path belongs to another install")
    }
    let digest
    if (["remove", "repair", "start"].includes(action)) {
      const marker = JSON.parse(await regular(join(root, "install.json")))
      digest = marker.releaseDigest
    } else digest = await prepareRelease({ stage, version, platform, releaseBase, publicKeyHex })
    const request = { action: action === "start" ? "repair" : action, installId, port, releaseDigest: digest }
    const adapter = bundleAdapter({ version, platform, extractorSource })
    const deps = { home, stage, releaseAdapter: adapter, ...(platform.startsWith("darwin") ? { serviceDefinition: launchdDefinition(installId), serviceManager: serviceManager ?? launchdManager(home, installId) } : serviceManager ? { serviceManager } : {}), ...(kernelCommand ? { kernelCommand } : {}) }
    const installed = await runMachine(request, { ...deps, enrollKernel: async () => { throw new Error("start requires enrollment") } })
    if (action === "remove") { await cliLink(home, root, installId, true); return installed }
    await cliLink(home, root, installId)
    if (options.installOnly || action === "upgrade") return installed
    const env = kernelEnv(home, installId, port)
    const enrollKernel = async invoke => {
      if (ticket) {
        const input = { ticket, apiUrl, ...(userId ? { userId } : {}) }
        try { return await invoke(["--owner-managed-self-enroll-stdin"], input) }
        finally { input.ticket = ""; options.ticket = "" }
      }
      return deviceEnroll(`${root}/current/bin/chariox-kernel`, { apiUrl, ...(userId ? { userId } : {}) }, env, openBrowser, notice)
    }
    return await runMachine({ ...request, releaseDigest: installed.releaseDigest ?? digest, action: "start" }, { ...deps, enrollKernel })
  } finally { options.ticket = ""; await rm(stage, { recursive: true, force: true }) }
}
