// MP-07 / MP-08 / MP-11: target-side per-user installation. Enrollment is a separate gate.
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { lstat, mkdir, mkdtemp, readFile, readdir, readlink, realpath, rename, rm, symlink, writeFile } from "node:fs/promises"
import { createServer } from "node:net"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import { recoverUpgrade, upgradeRelease } from "./upgrade.mjs"

const FORMAT = "chariox.ssh-machine-install.v1"
const here = dirname(fileURLToPath(import.meta.url))
const digestOf = bytes => `sha256:${createHash("sha256").update(bytes).digest("hex")}`
const fail = message => { throw new Error(message) }
export function validateRequest(r) {
  if (!r || typeof r !== "object" || Array.isArray(r) || Object.keys(r).some(k => !["action", "installId", "port", "releaseDigest"].includes(k))) fail("invalid SSH install request")
  if (!["install", "start", "stop", "remove", "upgrade", "repair", "inspect"].includes(r.action)) fail("invalid SSH install action")
  if (!/^[a-z][a-z0-9-]{0,47}$/.test(r.installId ?? "")) fail("install ID must be 1-48 lowercase letters, digits or hyphens")
  if (!Number.isInteger(r.port) || r.port < 1024 || r.port > 65534 || [43117, 43118, 43119, 43120].includes(r.port)) fail("choose a distinct unprivileged loopback port, other than the ordinary 43118 default")
  if (!/^sha256:[a-f0-9]{64}$/.test(r.releaseDigest ?? "")) fail("a pinned signed release digest is required")
}
function installLockDiagnostic(lock) {
  return `another install operation owns this install ID: ${lock}; wait for it to finish. If interrupted, first confirm no installer for this install ID is running, then remove only this empty lock directory and retry the same Setup or SSH command.`
}
function installLocked(lock) {
  throw Object.assign(new Error(installLockDiagnostic(lock)), { exitCode: 75 })
}
async function metadata(path) {
  return lstat(path).catch(e => e.code === "ENOENT" ? null : Promise.reject(e))
}
async function directory(path, create) {
  let m = await metadata(path)
  if (!m && create) { await mkdir(path, { mode: 0o700 }); m = await lstat(path) }
  if (!m || !m.isDirectory() || m.isSymbolicLink() || m.uid !== process.getuid() || (m.mode & 0o022)) fail("install path must be a user-owned directory without links or shared write access")
}
export async function tree(home, relative, create = true) {
  let p = home
  for (const part of relative.split("/")) { p = join(p, part); await directory(p, create) }
  return p
}
async function existingTree(home, relative) {
  let p = home
  for (const part of relative.split("/")) {
    p = join(p, part)
    if (!await metadata(p)) return null
    await directory(p, false)
  }
  return p
}
export async function regular(path, max = 65536) {
  const m = await lstat(path)
  if (!m.isFile() || m.isSymbolicLink() || m.uid !== process.getuid() || (m.mode & 0o022) || m.size > max) fail("install control file must be a bounded user-owned regular file")
  return readFile(path)
}
export async function command(program, args, capture = false, input, env = process.env) {
  const child = spawn(program, args, { env, stdio: [input ? "pipe" : "ignore", capture ? "pipe" : "ignore", "ignore"] })
  if (input) { child.stdin.on("error", () => {}); child.stdin.end(JSON.stringify(input)) }
  let out = "", failure
  const stop = () => {
    if (child.exitCode !== null || child.signalCode !== null) return
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) fail("refusing unsafe installer process signal")
    child.kill("SIGKILL")
  }
  if (capture) child.stdout.on("data", chunk => {
    if (out.length + chunk.length > 8192) { failure = new Error("service response exceeded limit"); stop() }
    else out += chunk.toString("utf8")
  })
  const timer = setTimeout(() => { failure = new Error("target install command timed out"); stop() }, 300_000)
  try {
    await new Promise((yes, no) => {
      child.once("error", () => no(new Error("target install prerequisite missing")))
      child.once("close", code => code === 0 && !failure ? yes() : no(failure ?? new Error("target install command failed")))
    })
    return out
  } finally { clearTimeout(timer) }
}
function quote(value) {
  if (typeof value !== "string" || /[\x00-\x1f\x7f]/.test(value)) fail("invalid user service environment")
  return `"${value.replaceAll("%", "%%").replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`
}
// MP-07 / MP-11: an install must not inherit another kernel's state or transport.
export const isolatedKernelEnvironment = ["CHARIOX_MANAGED_PROVIDER_TOPOLOGY", "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT", "CHARIOX_CAPABILITY_ISOLATION_ROOT", "CHARIOX_MANAGED_PROVIDER_ISOLATION", "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE", "CHARIOX_MANAGED_PROVIDER_BWRAP", "CHARIOX_MANAGED_PROVIDER_HOME", "CHARIOX_SLICE_ROOT", "CHARIOX_SLICE_DOCKER_BROKER_SOCKET", "CHARIOX_SLICE_DOCKER_BROKER_FD", "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED", "CHARIOX_RELAY_TOKEN", "CHARIOX_CLOUD_RELAY_CONFIG_JSON", "CHARIOX_CLOUD_RELAY_CONFIG_PATH", "CHARIOX_DAEMON_ID", "CHARIOX_MACHINE_ID", "CHARIOX_DAEMON_ALIAS", "CHARIOX_MACHINE_ALIAS", "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN", "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", "CHARIOX_LOG_DIR", "CHARIOX_PUBLICATION_CONTROL_STATE_DIR", "CHARIOX_DAEMON_SOCKET", "CHARIOX_SESSION_HISTORY_DIR", "CHARIOX_RELAY_URL", "CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY", "CHARIOX_KERNEL_RUNTIME_ROLE", "CHARIOX_REMOTE_LEASE_CAPACITY", "CHARIOX_LEASE_WORKER_HOME_CALLER", "CHARIOX_SLICE_OWNER_MACHINE_ID", "CHARIOX_AEDS_URL", "CHARIOX_AEDS_TOKEN", "CHARIOX_EVENT_ENVIRONMENT_ID", "CHARIOX_EVENT_REGISTRY_URL", "CHARIOX_AEGS_MANAGEMENT_TARGETS_FILE", "CHARIOX_AEGS_MANAGEMENT_TARGETS_JSON"]
export function unitFor(home, r, root, kernelPath = "usr/local/bin/chariox-kernel") {
  return `[Unit]\nDescription=Chariox SSH machine ${r.installId}\n\n[Service]\nType=simple\nEnvironment=${quote(`HOME=${home}`)}\nEnvironment=${quote(`CHARIOX_HOME=${home}/.chariox/dev/ssh-machines/${r.installId}`)}\nEnvironment=${quote(`PATH=${process.env.PATH}`)}\nEnvironment=CHARIOX_KERNEL_HOST=127.0.0.1\nEnvironment=CHARIOX_KERNEL_PORT=${r.port}\nEnvironment=CHARIOX_MCP_HOST=127.0.0.1\nEnvironment=CHARIOX_MCP_PORT=${r.port + 1}\nUnsetEnvironment=${isolatedKernelEnvironment.join(" ")}\nExecStart=${quote(`${root}/current/${kernelPath}`)}\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n`
}
async function verifyImage(image, digest, pins, stage) {
  await command("node", [join(stage, "verify-image-release.mjs"), image, digest, join(pins, "release-public-pin"), "path1", join(pins, "builder-public-pin")])
  // The managed verifier hashes every declared artifact; reject additional, unsigned files too.
  const manifest = JSON.parse(await readFile(join(image, "usr/lib/chariox/release-manifest.json")))
  const files = new Set(manifest.artifacts.filter(a => a.name !== "chariox-slice-build-context").map(a => a.path.slice(1)))
  for (const path of ["release-manifest.json", "release-manifest.sig", "release-public-key"]) files.add(`usr/lib/chariox/${path}`)
  const context = "usr/lib/chariox/slice-build-context"
  async function walk(dir, prefix = "") {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const p = prefix ? `${prefix}/${entry.name}` : entry.name
      if (!(files.has(p) || [...files].some(f => f.startsWith(`${p}/`)) || p === context || p.startsWith(`${context}/`) || context.startsWith(`${p}/`))) fail("unexpected unsigned release material")
      if (entry.isDirectory()) await walk(join(dir, entry.name), p)
    }
  }
  await walk(image)
}
async function portFree(port) {
  const server = createServer()
  await new Promise((yes, no) => {
    server.once("error", () => no(new Error("selected loopback port is already occupied")))
    server.listen({ host: "127.0.0.1", port, exclusive: true }, yes)
  })
  await new Promise((yes, no) => server.close(e => e ? no(e) : yes()))
}
function kernelEnvironment(home, stateRelative, r) {
  const env = { ...process.env, HOME: home, CHARIOX_HOME: `${home}/${stateRelative}`, CHARIOX_KERNEL_HOST: "127.0.0.1", CHARIOX_KERNEL_PORT: String(r.port), CHARIOX_MCP_HOST: "127.0.0.1", CHARIOX_MCP_PORT: String(r.port + 1) }
  for (const key of Object.keys(env)) if (key.startsWith("CHARIOX_") && !["CHARIOX_HOME", "CHARIOX_KERNEL_HOST", "CHARIOX_KERNEL_PORT", "CHARIOX_MCP_HOST", "CHARIOX_MCP_PORT"].includes(key)) delete env[key]
  return env
}
// MP-07 / MP-08 / MP-11: service mutations belong only to this invocation.
async function serviceState(serviceManager, service) {
  const info = await serviceManager(["show", service, "--property=ActiveState", "--property=UnitFileState"], true)
  const fields = Object.fromEntries(info.trim().split("\n").map(line => line.split(/=(.*)/s).slice(0, 2)))
  if (!["active", "inactive", "failed"].includes(fields.ActiveState) || !["enabled", "disabled", "not-found", ""].includes(fields.UnitFileState)) fail("cannot prove prior service state")
  return { active: fields.ActiveState === "active", enabled: fields.UnitFileState === "enabled" }
}
// A durable removal intent is written only after validating and stopping the owned install.
async function pendingRemoval(root, marker, serviceManager) {
  const path = join(root, "removal.json")
  if (!await metadata(path)) return false
  if (JSON.stringify(JSON.parse(await regular(path))) !== JSON.stringify(marker)) fail("removal identity differs from owned install")
  const state = await serviceState(serviceManager, marker.service)
  if (state.active || state.enabled) fail("pending removal requires an owned stopped, disabled service")
  return true
}
// MP-08 / MP-11: absence is a read-only target fact, never inferred from an SSH failure.
async function inspectMachine(r, home, stage, serviceManager) {
  const parent = await existingTree(home, ".local/share/chariox/ssh-machines")
  if (parent && await metadata(join(parent, `.${r.installId}.lock`))) installLocked(join(parent, `.${r.installId}.lock`))
  const unitDir = await existingTree(home, ".config/systemd/user")
  const root = parent ? join(parent, r.installId) : null
  const service = `chariox-ssh-${r.installId}.service`, unit = unitDir ? join(unitDir, service) : null
  const info = await serviceManager(["show",service,"--property=LoadState","--property=FragmentPath","--property=DropInPaths"],true)
  const fields = Object.fromEntries(info.trim().split("\n").map(line => line.split(/=(.*)/s).slice(0,2)))
  if (!Object.hasOwn(fields,"DropInPaths") || !Object.hasOwn(fields,"FragmentPath") || fields.DropInPaths) fail("cannot prove install ownership")
  if (!root || !await metadata(root)) {
    if (fields.LoadState !== "not-found" || fields.FragmentPath || (unit && await metadata(unit))) fail("unit exists without an owned release root")
    return { installId:r.installId,status:"absent" }
  }
  await directory(root,false)
  const marker = JSON.parse(await regular(join(root,"install.json")))
  if (marker.format !== FORMAT || marker.installId !== r.installId || marker.port !== r.port || marker.service !== service || marker.releaseDigest !== r.releaseDigest || !unit || (fields.FragmentPath && fields.FragmentPath !== unit)) fail("existing install differs")
  const removing = await pendingRemoval(root, marker, serviceManager)
  if ((!removing || await metadata(unit)) && digestOf(await regular(unit)) !== marker.unitDigest) fail("existing install was changed")
  if (await readlink(join(root,"current")) !== `releases/${r.releaseDigest.slice(7)}`) fail("existing install was changed")
  await directory(join(root,"releases"),false); await directory(join(root,"releases",r.releaseDigest.slice(7)),false)
  await verifyImage(join(root,"releases",r.releaseDigest.slice(7)),r.releaseDigest,root,stage)
  return { installId:r.installId,status:"installed",releaseDigest:r.releaseDigest,enrolled:false }
}
// serviceManager is a test seam, never supplied by serialized requests or environment flags.
export async function runMachine(r, { home = process.env.HOME, stage = here, enrollment, enrollKernel, kernelCommand, releaseAdapter, serviceDefinition, serviceManager = (args, capture) => command("systemctl", ["--user", ...args], capture) } = {}) {
  validateRequest(r)
  const verify = releaseAdapter?.verify ?? verifyImage
  const kernelPath = releaseAdapter?.kernelPath ?? "usr/local/bin/chariox-kernel"
  const manifestPath = releaseAdapter?.manifestPath ?? "usr/lib/chariox/release-manifest.json"
  const installing = ["install", "upgrade", "repair"].includes(r.action)
  if (r.action === "start" && !enrollment && !enrollKernel) fail("bounded owner-managed enrollment input required")

  if (!home || !home.startsWith("/") || home === "/" || resolve(home) !== home || await realpath(home) !== home) fail("user HOME must be an absolute canonical non-root directory")
  await directory(home, false)
  if (r.action === "inspect") {
    if (releaseAdapter || serviceDefinition) fail("SSH inspection requires the original image/service contract")
    return inspectMachine(r, home, stage, serviceManager)
  }
  const service = serviceDefinition?.name ?? `chariox-ssh-${r.installId}.service`
  const renderUnit = () => serviceDefinition?.render(home, r, root, kernelPath) ?? unitFor(home, r, root, kernelPath)
  const installParent = await tree(home, ".local/share/chariox/ssh-machines", installing)
  const root = join(installParent, r.installId)
  const unitDir = await tree(home, serviceDefinition?.relativeDirectory ?? ".config/systemd/user", installing)
  const unitPath = join(unitDir, service)
  const markerPath = join(root, "install.json")
  const stateRelative = `.chariox/dev/ssh-machines/${r.installId}`
  const oldState = await existingTree(home, stateRelative)
  if (oldState) {
    const m = await metadata(join(oldState, "ssh-install-owner.json"))
    if (!m) fail("unmarked state belongs to another installation")
    const binding = JSON.parse(await regular(join(oldState, "ssh-install-owner.json"), 4096))
    if (binding.format !== FORMAT || binding.installId !== r.installId || binding.port !== r.port || binding.service !== service) fail("kernel state belongs to another installation")
  }
  const lock = join(installParent, `.${r.installId}.lock`)
  await mkdir(lock, { mode: 0o700 }).catch(error => {
    if (error.code !== "EEXIST") throw error
    installLocked(lock)
  })
  let scratch
  try {
    const info = await serviceManager(["show", service, "--property=LoadState", "--property=FragmentPath", "--property=DropInPaths"], true)
    const fields = Object.fromEntries(info.trim().split("\n").map(line => line.split(/=(.*)/s).slice(0, 2)))
    if (!["not-found", "loaded"].includes(fields.LoadState) || !Object.hasOwn(fields, "FragmentPath") || !Object.hasOwn(fields, "DropInPaths") || fields.DropInPaths) fail("working systemd --user with no unit overrides is required")
    const rootMeta = await metadata(root)
    let marker, removing = false
    if (rootMeta) {
      await directory(root, false)
      const expected = ["builder-public-pin", "current", "install.json", "release-public-pin", "releases"]
      if (await metadata(join(root, "upgrade.json"))) {
        expected.push("upgrade.json")
        for (const name of [".current-new", "install.json.new"]) if (await metadata(join(root, name))) expected.push(name)
      }
      if (await metadata(join(root, "removal.json"))) expected.push("removal.json")
      expected.sort()
      if (JSON.stringify((await readdir(root)).sort()) !== JSON.stringify(expected)) fail("unexpected material in install root; retain it and resolve ownership before uninstall")
      marker = JSON.parse(await regular(markerPath))
      if (marker.format !== FORMAT || marker.installId !== r.installId || marker.service !== service || marker.port !== r.port || (marker.releaseDigest !== r.releaseDigest && r.action !== "upgrade")) fail("existing install identity differs; use explicit upgrade rather than replacing it")
      if (fields.FragmentPath && fields.FragmentPath !== unitPath) fail("service name belongs to another installation")
      removing = await pendingRemoval(root, marker, serviceManager)
      if (removing && r.action !== "remove") fail("removal in progress; retry remove before reinstalling")
      const missingUnit = !await metadata(unitPath)
      if (!(missingUnit && (removing || r.action === "repair")) && digestOf(await regular(unitPath)) !== marker.unitDigest) fail("user service was changed; refusing to control it")
      await directory(join(root, "releases"), false)
      const releases = await readdir(join(root, "releases"))
      if (releases.some(digest => !/^[a-f0-9]{64}$/.test(digest))) fail("unexpected release material")
      marker = await recoverUpgrade({ root, markerPath, marker, r, service, stage, verify, serviceManager })
      const current = await lstat(join(root, "current"))
      if (!current.isSymbolicLink() || await readlink(join(root, "current")) !== `releases/${marker.releaseDigest.slice(7)}`) fail("active release pointer was changed")
      await directory(join(root, "releases"), false)
      await directory(join(root, "releases", marker.releaseDigest.slice(7)), false)
      if (r.action !== "stop") {
        await verify(join(root, "releases", marker.releaseDigest.slice(7)), marker.releaseDigest, root, stage)
      }
      if (!await metadata(unitPath) && r.action === "repair") {
        const unit = marker.unitContent ?? renderUnit()
        if (typeof unit !== "string" || Buffer.byteLength(unit) > 65536) fail("invalid retained service definition")
        if (digestOf(Buffer.from(unit)) !== marker.unitDigest) fail("repair cannot change service policy")
        await writeFile(unitPath, unit, { mode: 0o600, flag: "wx" })
        await serviceManager(["daemon-reload"])
      }
    } else if (await metadata(unitPath) || fields.LoadState !== "not-found" || fields.FragmentPath) {
      fail("service name belongs to another installation")
    }
    if (r.action === "stop" || r.action === "remove") {
      if (!marker) fail("no owned install found")
      if (!removing) await serviceManager(["disable", "--now", service])
      if (r.action === "remove") {
        for (const digest of await readdir(join(root, "releases"))) await verify(join(root, "releases", digest), `sha256:${digest}`, root, stage)
        if (!removing) {
          const state = await serviceState(serviceManager, service)
          if (state.active || state.enabled) fail("removal requires an owned stopped, disabled service")
          await writeFile(join(root, "removal.json"), JSON.stringify(marker), { mode: 0o600, flag: "wx" })
        }
        if (await metadata(unitPath)) await rm(unitPath)
        await serviceManager(["daemon-reload"])
        // This marked root contains release bytes/public pins only; mutable runtime state is separate and retained.
        await rm(root, { recursive: true })
      }
      return { installId: r.installId, status: r.action === "stop" ? "stopped" : "removed", stateRetained: true }
    }
    if (r.action === "start") {
      if (!marker) fail("no owned install found")
      if (!enrollKernel && (!enrollment || Object.keys(enrollment).sort().join(",") !== "apiUrl,ticket,userId" || typeof enrollment.ticket !== "string" || !enrollment.ticket || enrollment.ticket.length > 4096 || typeof enrollment.userId !== "string" || !enrollment.userId)) fail("bounded owner-managed enrollment input required")
      const env = kernelEnvironment(home, stateRelative, r)
      const invoke = kernelCommand ?? (async (args, input) => JSON.parse(await command(join(root, "current", kernelPath), args, true, input, env)))
      const identity = enrollKernel ? await enrollKernel(invoke) : await invoke(["--owner-managed-enroll-stdin"], enrollment)
      if (enrollment) enrollment.ticket = ""
      if (!identity || (enrollment && identity.userId !== enrollment.userId) || !identity.kernelId || !identity.machineId || !identity.publicKeyThumbprint) fail("target enrollment identity does not match the owner")
      const prior = await serviceState(serviceManager, service)
      let enabledHere = false, startedHere = false
      try {
        if (!prior.enabled) { enabledHere = true; await serviceManager(["enable", service]) }
        if (!prior.active) { startedHere = true; await serviceManager(["start", service]) }
        const ready = await invoke(["--owner-managed-ready"])
        if (!ready.connected || ["kernelId", "machineId", "userId", "publicKeyThumbprint"].some(k => ready[k] !== identity[k])) fail("target kernel did not become relay-ready with the enrolled identity")
        return { installId: r.installId, status: "ready", releaseDigest: r.releaseDigest, ...Object.fromEntries(["kernelId", "machineId", "userId", "publicKeyThumbprint"].map(k => [k, identity[k]])), ticketConsumed: identity.ticketConsumed === true, connected: true }
      } catch (error) {
        try { if (startedHere) await serviceManager(["stop", service]) }
        finally { if (enabledHere) await serviceManager(["disable", service]) }
        throw error
      }
    }
    if (marker && (r.action !== "upgrade" || marker.releaseDigest === r.releaseDigest)) return { installId: r.installId, status: "installed", releaseDigest: marker.releaseDigest, enrolled: false }
    if (!marker) { await portFree(r.port); await portFree(r.port + 1) }
    scratch = await mkdtemp(join(installParent, `.${r.installId}.stage-`))
    const image = join(scratch, "image")
    if (releaseAdapter?.extract) await releaseAdapter.extract(image, stage)
    else await command("python3", [join(stage, "extract-release.py"), join(stage, "release.tar.gz"), image])
    // An upgrade trusts the original independent pins; changing keys is not upgrading.
    await verify(image, r.releaseDigest, marker ? root : stage, stage)
    const protocol = (await command(join(image, kernelPath), ["--print-local-daemon-protocol-version"], true)).trim()
    if (!/^\d+$/.test(protocol) || Number(protocol) < 479) fail("chosen release requires owner-managed bootstrap protocol 479 or newer")
    const manifest = JSON.parse(await readFile(join(image, manifestPath)))
    const unit = renderUnit()
    // Keep staging and the exclusive install lock through activation or rollback.
    if (marker) return await upgradeRelease({ root, markerPath, marker, r, image, manifest, stage, verify, serviceManager, kernelCommand, kernelPath, env: kernelEnvironment(home, stateRelative, r), service })
    const state = await tree(home, stateRelative)
    if (!oldState) await writeFile(join(state, "ssh-install-owner.json"), JSON.stringify({ format: FORMAT, installId: r.installId, port: r.port, service }), { flag: "wx", mode: 0o600 })
    const pending = join(scratch, "install")
    await mkdir(pending, { mode: 0o700 })
    await mkdir(join(pending, "releases"), { mode: 0o700 })
    for (const name of ["release-public-pin", "builder-public-pin"]) {
      await writeFile(join(pending, name), await regular(join(stage, name), 1024), { mode: 0o600, flag: "wx" })
    }
    await rename(image, join(pending, "releases", r.releaseDigest.slice(7)))
    await symlink(`releases/${r.releaseDigest.slice(7)}`, join(pending, "current"))
    await writeFile(join(pending, "install.json"), JSON.stringify({ format: FORMAT, installId: r.installId, service, port: r.port, releaseDigest: r.releaseDigest, unitDigest: digestOf(Buffer.from(unit)), unitContent: unit, sourceCommit: manifest.sourceCommit, sourceTree: manifest.sourceTree }), { mode: 0o600, flag: "wx" })
    // Exclusively claim the unit before publishing the marked root. Refuse a competing unit/root.
    await writeFile(unitPath, unit, { mode: 0o600, flag: "wx" })
    try {
      if (await metadata(root)) fail("install root appeared during publication")
      await rename(pending, root)
    } catch (error) { await rm(unitPath); throw error }
    await serviceManager(["daemon-reload"])
    return { installId: r.installId, status: "installed", releaseDigest: r.releaseDigest, enrolled: false }
  } finally {
    if (scratch) await rm(scratch, { recursive: true, force: true })
    await rm(lock, { recursive: true })
  }
}
if (typeof Bun === "undefined" && process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (Number(process.versions.node.split(".")[0]) < 22) fail("Node >=22 required")
    if (process.argv.length !== 3) fail("one bounded installer input required")
    let r, enrollment
    if (process.argv[2] === "--stdin") {
      let bytes = Buffer.alloc(0)
      for await (const chunk of process.stdin) { if (bytes.length + chunk.length > 8192) fail("bootstrap input exceeded limit"); bytes = Buffer.concat([bytes, chunk]) }
      const input = JSON.parse(bytes.toString("utf8")); bytes.fill(0)
      r = input.request; enrollment = input.enrollment
    } else r = JSON.parse(await regular(process.argv[2], 4096))
    try { process.stdout.write(`${JSON.stringify(await runMachine(r, { enrollment }))}\n`) }
    finally { if (enrollment) enrollment.ticket = "" }
  } catch (error) {
    // Target errors can contain paths/profile output. The home exposes only a bounded generic failure.
    process.stderr.write("MP-07/MP-08/MP-11: SSH machine installation refused\n")
    process.exitCode = error?.exitCode === 75 ? 75 : 1
  }
}
