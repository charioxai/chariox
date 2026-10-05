// MP-07 / MP-08 / MP-11: home-kernel SSH deployment transport. No credentials.
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { createReadStream } from "node:fs"
import { lstat, readFile } from "node:fs/promises"
import { Readable } from "node:stream"
import { fileURLToPath } from "node:url"

const assets = [
  ["remote.mjs", new URL("./remote.mjs", import.meta.url)],
  ["extract-release.py", new URL("../../../deploy/managed-kernel/extract-release.py", import.meta.url)],
  ["verify-image-release.mjs", new URL("../../../deploy/managed-kernel/verify-image-release.mjs", import.meta.url)],
  ["path1-service-policy.mjs", new URL("../../../deploy/managed-kernel/path1-service-policy.mjs", import.meta.url)],
]
export function validateDestination(host) {
  if (typeof host !== "string" || host.length > 255 || !/^[a-zA-Z0-9_][a-zA-Z0-9_.@:[\]-]*$/.test(host)) {
    throw new Error("SSH destination must be a host/config alias, optionally user@host; options and shell syntax are refused")
  }
  return host
}
export function sshArguments(host, remoteCommand) {
  return ["-oBatchMode=yes", "-oClearAllForwardings=yes", "-oForwardAgent=no", "--", validateDestination(host), remoteCommand]
}
function guardPid(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error("refusing unsafe SSH process signal")
}
// Only signal a ChildProcess this operation spawned; never a process group.
function stopChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  guardPid(child.pid)
  child.kill("SIGKILL")
}
async function ssh(host, command, input, { signal, timeoutMs = 300_000 } = {}) {
  if (signal?.aborted) throw new Error("SSH operation cancelled")
  const child = spawn("ssh", sshArguments(host, command), { stdio: ["pipe", "pipe", "pipe"] })
  let output = "", bytes = 0, failed
  // Remote/provider output is never reflected in errors or logs. Only safe JSON stdout is returned.
  child.stderr.on("data", () => {})
  child.stdout.on("data", chunk => {
    bytes += chunk.length
    if (bytes > 8192) { failed = new Error("SSH response exceeded limit"); stopChild(child) }
    else output += chunk.toString("utf8")
  })
  const cancel = () => { failed = new Error("SSH operation cancelled"); stopChild(child) }
  const timer = setTimeout(() => { failed = new Error("SSH operation timed out"); stopChild(child) }, timeoutMs)
  signal?.addEventListener("abort", cancel, { once: true })
  const closed = new Promise((resolve, reject) => {
    child.once("error", reject)
    child.once("close", (code) => code === 0 && !failed ? resolve(output) : reject(failed ?? new Error("SSH deployment failed; check SSH access, target prerequisites and install ownership")))
  })
  // Attach immediately: close can arrive while the upload is still producing.
  closed.catch(() => {})
  const stream = Readable.from(input)
  stream.on("error", () => { failed = new Error("SSH upload failed"); if (child.pid !== undefined) stopChild(child) })
  child.stdin.on("error", () => stream.destroy())
  stream.pipe(child.stdin)
  try { return await closed }
  finally { clearTimeout(timer); signal?.removeEventListener("abort", cancel); stream.destroy() }
}
function header(name, size) {
  const b = Buffer.alloc(512)
  b.write(name, 0, 100, "ascii")
  for (const [at, width, value] of [[100,8,0o600],[108,8,0],[116,8,0],[124,12,size],[136,12,0]]) {
    const octal = value.toString(8)
    if (octal.length >= width) throw new Error("upload entry too large")
    b.write(octal.padStart(width - 1, "0") + "\0", at, width, "ascii")
  }
  b.fill(32, 148, 156); b[156] = 48
  b.write("ustar\0", 257, 6); b.write("00", 263, 2)
  const sum = b.reduce((a, c) => a + c, 0)
  b.write(sum.toString(8).padStart(6, "0") + "\0 ", 148, 8)
  return b
}
async function* upload(entries) {
  for (const [name, data] of entries) {
    const isBytes = Buffer.isBuffer(data)
    const stat = isBytes ? null : await lstat(data)
    if (stat && (!stat.isFile() || stat.isSymbolicLink() || stat.size > 2 * 1024 ** 3)) throw new Error("upload requires a bounded regular file")
    const size = isBytes ? data.length : stat.size
    yield header(name, size)
    let seen = 0
    if (isBytes) { yield data; seen = size }
    else for await (const chunk of createReadStream(data)) { seen += chunk.length; if (seen > size) throw new Error("upload file changed"); yield chunk }
    if (seen !== size) throw new Error("upload file changed")
    yield Buffer.alloc((512 - size % 512) % 512)
  }
  yield Buffer.alloc(1024)
}
async function admittedPublicPin(path, expected) {
  const m = await lstat(path)
  if (!m.isFile() || m.isSymbolicLink() || m.size > 1024) throw new Error("independent public pin must be a bounded regular file")
  const bytes = await readFile(path), text = bytes.toString("utf8").trim(), raw = Buffer.from(text, "base64")
  const actual = `sha256:${createHash("sha256").update(raw).digest("hex")}`
  if (raw.length !== 32 || raw.toString("base64") !== text || !/^sha256:[a-f0-9]{64}$/.test(expected ?? "") || actual !== expected) throw new Error("public pin fingerprint does not match the approved inventory")
  return Buffer.from(text)
}
export async function runSshMachine(host, request, release, options = {}) {
  validateDestination(host)
  const { validateRequest } = await import("./remote.mjs")
  validateRequest(request)
  let pins
  if (request.action === "install") {
    if (!release) throw new Error("signed release and independent public pins required")
    pins = [await admittedPublicPin(release.releasePublicKey, release.releasePublicKeyFingerprint), await admittedPublicPin(release.builderPublicKey, release.builderPublicKeyFingerprint)]
  }
  const platform = (await ssh(host, "uname -s && uname -m", [], options)).trim().split(/\r?\n/)
  if (platform.length !== 2 || platform[0] !== "Linux" || platform[1] !== "x86_64") throw new Error("BYOM PR1 requires Linux x86_64")
  const entries = [["request.json", Buffer.from(JSON.stringify(request))]]
  for (const [name, path] of assets) entries.push([name, await readFile(fileURLToPath(path))])
  if (request.action === "install") {
    entries.push(["release.tar.gz", release.archive], ["release-public-pin", pins[0]], ["builder-public-pin", pins[1]])
  }
  // All outer members are home-owned fixed names; the untrusted inner archive is bounded by the shared extractor.
  const command = `set -eu; umask 077; stage=$(mktemp -d \"$HOME/.chariox-byom-upload.XXXXXXXX\"); trap 'rm -rf -- \"$stage\"' EXIT HUP INT TERM; tar -xf - -C \"$stage\" --no-same-owner; node \"$stage/remote.mjs\" \"$stage/request.json\"`
  let raw
  if (request.action === "start") {
    // Public helper bytes only in the command. The ticket remains a bounded stdin JSON frame.
    const writes = entries.filter(([name]) => name !== "request.json").map(([name, bytes]) => `printf '%s' '${bytes.toString("base64")}' | base64 -d > "$stage/${name}"`).join("; ")
    const start = `set -eu; umask 077; stage=$(mktemp -d "$HOME/.chariox-byom-upload.XXXXXXXX"); trap 'rm -rf -- "$stage"' EXIT HUP INT TERM; ${writes}; node "$stage/remote.mjs" --stdin`
    raw = await ssh(host, start, [Buffer.from(JSON.stringify({ request, enrollment: options.enrollment }))], options)
  } else raw = await ssh(host, command, upload(entries), options)
  let result
  try { result = JSON.parse(raw) } catch { throw new Error("SSH target returned an invalid deployment result") }
  if (result?.installId !== request.installId || !["installed", "ready", "stopped", "removed", "absent"].includes(result?.status)) throw new Error("SSH deployment result identity is invalid")
  if (request.action === "inspect") {
    if (!["absent", "installed"].includes(result.status)) throw new Error("invalid target inspection")
    return { installId:request.installId,status:result.status }
  }
  const expected = { install: "installed", start: "ready", stop: "stopped", remove: "removed" }[request.action]
  if (result.status !== expected || (expected === "installed" && (result.releaseDigest !== request.releaseDigest || result.enrolled !== false))) throw new Error("SSH deployment result does not match the requested action")
  if (expected === "ready") {
    if (result.releaseDigest !== request.releaseDigest || result.userId !== options.enrollment?.userId || result.connected !== true || ["kernelId", "machineId", "publicKeyThumbprint"].some(k => typeof result[k] !== "string" || !result[k] || result[k].length > 512)) throw new Error("SSH target did not return bound relay readiness")
    return { installId: request.installId, status: "ready", releaseDigest: result.releaseDigest, kernelId: result.kernelId, machineId: result.machineId, userId: result.userId, publicKeyThumbprint: result.publicKeyThumbprint, ticketConsumed: result.ticketConsumed === true, connected: true }
  }
  return expected === "installed"
    ? { installId: request.installId, status: expected, releaseDigest: request.releaseDigest, enrolled: false }
    : { installId: request.installId, status: expected, stateRetained: true }
}
