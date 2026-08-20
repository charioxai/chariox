import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, verify } from "node:crypto"
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, relative } from "node:path"
import { fileURLToPath } from "node:url"
import { spawnSync } from "node:child_process"
import { test } from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const packager = join(repositoryRoot, "scripts/package-managed-kernel-release.mjs")
const installer = join(repositoryRoot, "deploy/managed-kernel/install-image.sh")
const verifier = join(repositoryRoot, "deploy/managed-kernel/verify-image-release.mjs")
const service = join(repositoryRoot, "deploy/managed-kernel/chariox-managed-bootstrap.service")
const sourceDateEpoch = "946684800"

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: "der", type: "spki" })
  return der.subarray(der.length - 32)
}

function packagerArguments({ kernel, supervisor, signingKey, output }) {
  return [packager, "--kernel", kernel, "--supervisor", supervisor, "--signing-key", signingKey, "--output", output]
}

function runPackager(options, umask = "022") {
  return spawnSync(
    "/bin/sh",
    ["-c", 'umask "$1"; shift; exec "$@"', "chariox-release-packager", umask, process.execPath, ...packagerArguments(options)],
    { encoding: "utf8", env: { ...process.env, SOURCE_DATE_EPOCH: sourceDateEpoch } },
  )
}

function runVerifier(rootfs, digest, trustedPublicKey) {
  return spawnSync(process.execPath, [verifier, rootfs, digest, trustedPublicKey], { encoding: "utf8" })
}

async function snapshotTree(root, current = root) {
  const metadata = await lstat(current)
  const record = {
    path: relative(root, current) || ".",
    mode: metadata.mode & 0o777,
    mtimeMs: metadata.mtimeMs,
    type: metadata.isDirectory() ? "directory" : metadata.isFile() ? "file" : "unsupported",
  }
  if (metadata.isFile()) record.sha256 = createHash("sha256").update(await readFile(current)).digest("hex")
  const records = [record]
  if (metadata.isDirectory()) {
    for (const name of (await readdir(current)).sort()) records.push(...(await snapshotTree(root, join(current, name))))
  }
  return records
}

async function makeFixture(root) {
  const kernel = join(root, "chariox-kernel")
  const supervisor = join(root, "chariox-managed-bootstrap")
  const signingKey = join(root, "release-key.pem")
  const trustedPublicKey = join(root, "trusted-release-public-key")
  await writeFile(kernel, "kernel fixture\n", { mode: 0o755 })
  await writeFile(supervisor, "supervisor fixture\n", { mode: 0o755 })
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  await writeFile(signingKey, privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  await writeFile(trustedPublicKey, rawPublicKey(publicKey).toString("base64"), { mode: 0o600 })
  return { kernel, supervisor, signingKey, trustedPublicKey, publicKey }
}

test("managed kernel release packages one reproducible signed rootfs", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const firstOutput = join(root, "release-one")
  const secondOutput = join(root, "release-two")
  const first = runPackager({ ...fixture, output: firstOutput }, "022")
  const second = runPackager({ ...fixture, output: secondOutput }, "077")
  assert.equal(first.status, 0, first.stderr)
  assert.equal(second.status, 0, second.stderr)
  assert.match(first.stdout, /^sha256:[a-f0-9]{64}\n$/)
  assert.equal(second.stdout, first.stdout)
  assert.equal(first.stderr, "")

  const releaseRoot = join(firstOutput, "rootfs")
  const secondReleaseRoot = join(secondOutput, "rootfs")
  assert.deepEqual(await snapshotTree(releaseRoot), await snapshotTree(secondReleaseRoot))
  const snapshot = await snapshotTree(releaseRoot)
  assert.equal(snapshot.every((entry) => entry.mtimeMs === Number(sourceDateEpoch) * 1000), true)
  assert.equal(snapshot.every((entry) => entry.type !== "unsupported"), true)
  assert.deepEqual(snapshot.filter((entry) => entry.type === "file").map((entry) => entry.path), [
    "etc/systemd/system/chariox-managed-bootstrap.service",
    "usr/lib/chariox/release-manifest.json",
    "usr/lib/chariox/release-manifest.sig",
    "usr/lib/chariox/release-public-key",
    "usr/local/bin/chariox-kernel",
    "usr/local/bin/chariox-managed-bootstrap",
  ])

  const manifestBytes = await readFile(join(releaseRoot, "usr/lib/chariox/release-manifest.json"))
  const manifest = JSON.parse(manifestBytes)
  const digest = (contents) => `sha256:${createHash("sha256").update(contents).digest("hex")}`
  assert.deepEqual(manifest, {
    schemaVersion: 1,
    artifacts: [
      { name: "chariox-kernel", path: "/usr/local/bin/chariox-kernel", sha256: digest("kernel fixture\n") },
      { name: "chariox-managed-bootstrap", path: "/usr/local/bin/chariox-managed-bootstrap", sha256: digest("supervisor fixture\n") },
      {
        name: "chariox-managed-bootstrap.service",
        path: "/etc/systemd/system/chariox-managed-bootstrap.service",
        sha256: digest(await readFile(service)),
      },
    ],
  })
  assert.equal(first.stdout.trim(), digest(manifestBytes))
  const signature = Buffer.from((await readFile(join(releaseRoot, "usr/lib/chariox/release-manifest.sig"), "utf8")).trim(), "base64")
  assert.equal(signature.length, 64)
  assert.equal(verify(null, manifestBytes, fixture.publicKey, signature), true)
  assert.deepEqual(
    Buffer.from((await readFile(join(releaseRoot, "usr/lib/chariox/release-public-key"), "utf8")).trim(), "base64"),
    rawPublicKey(fixture.publicKey),
  )
  assert.equal(await readFile(join(releaseRoot, "etc/systemd/system/chariox-managed-bootstrap.service"), "utf8"), await readFile(service, "utf8"))
  assert.equal(snapshot.find((entry) => entry.path === "usr/local/bin/chariox-kernel").mode, 0o755)
  assert.equal(snapshot.find((entry) => entry.path === "usr/local/bin/chariox-managed-bootstrap").mode, 0o755)
  assert.equal(
    snapshot.filter((entry) => entry.type === "file" && !entry.path.startsWith("usr/local/bin/")).every((entry) => entry.mode === 0o644),
    true,
  )
  assert.equal(snapshot.some((entry) => entry.path.includes("release-key")), false)
  assert.equal(runVerifier(releaseRoot, first.stdout.trim(), fixture.trustedPublicKey).status, 0)

  const rerun = runPackager({ ...fixture, output: firstOutput })
  assert.equal(rerun.status, 1)
  assert.match(rerun.stderr, /output directory must be empty/)
})

test("release identity binds the supervisor and verifier rejects tampering", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-binding-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const originalOutput = join(root, "original")
  const changedOutput = join(root, "changed")
  const original = runPackager({ ...fixture, output: originalOutput })
  assert.equal(original.status, 0, original.stderr)
  await writeFile(fixture.supervisor, "different supervisor\n", { mode: 0o755 })
  const changed = runPackager({ ...fixture, output: changedOutput })
  assert.equal(changed.status, 0, changed.stderr)
  assert.notEqual(changed.stdout, original.stdout)

  const originalRoot = join(originalOutput, "rootfs")
  await writeFile(join(originalRoot, "usr/local/bin/chariox-managed-bootstrap"), "tampered\n")
  const supervisorTamper = runVerifier(originalRoot, original.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(supervisorTamper.status, 1)
  assert.match(supervisorTamper.stderr, /chariox-managed-bootstrap is corrupted/)

  const changedRoot = join(changedOutput, "rootfs")
  await writeFile(join(changedRoot, "etc/systemd/system/chariox-managed-bootstrap.service"), "tampered service\n")
  const serviceTamper = runVerifier(changedRoot, changed.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(serviceTamper.status, 1)
  assert.match(serviceTamper.stderr, /chariox-managed-bootstrap\.service is corrupted/)
})

test("release verifier rejects root and key aliases into the packaged image", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-alias-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const rootfs = join(output, "rootfs")
  const rootfsAlias = join(root, "rootfs-alias")
  await symlink(rootfs, rootfsAlias)
  const rootAliasResult = runVerifier(rootfsAlias, packaged.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(rootAliasResult.status, 1)
  assert.match(rootAliasResult.stderr, /image root must be a directory, not a symlink/)

  const releaseDirectoryAlias = join(root, "release-directory-alias")
  await symlink(join(rootfs, "usr/lib/chariox"), releaseDirectoryAlias)
  const keyAliasResult = runVerifier(
    rootfs,
    packaged.stdout.trim(),
    join(releaseDirectoryAlias, "release-public-key"),
  )
  assert.equal(keyAliasResult.status, 1)
  assert.match(keyAliasResult.stderr, /trusted release public key must be supplied outside/)
})

test("managed kernel release packaging rejects symlink inputs", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-symlink-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const kernelLink = join(root, "kernel-link")
  await symlink(fixture.kernel, kernelLink)
  const result = runPackager({ ...fixture, kernel: kernelLink, output: join(root, "output") })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /kernel binary must be a bounded regular file/)
})

test("managed kernel release packaging rejects a broadly readable signing key", async (context) => {
  if (process.platform === "win32") return
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-key-mode-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await chmod(fixture.signingKey, 0o644)
  const result = runPackager({ ...fixture, output: join(root, "output") })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /must not be readable by group or other users/)
})

async function writeHarnessCommand(path, contents) {
  await writeFile(path, contents, { mode: 0o755 })
  await chmod(path, 0o755)
}

async function createInstallerHarness(root) {
  const bin = join(root, "bin")
  const state = join(root, "command-state")
  const installRoot = join(root, "installed")
  await mkdir(bin, { recursive: true })
  await mkdir(state, { recursive: true })
  await writeHarnessCommand(join(bin, "id"), `#!/bin/sh
if [ "\${1:-}" = "-u" ]; then echo 0; exit 0; fi
if [ "\${1:-}" = "-gn" ]; then [ -f "$HARNESS_STATE/user" ] && echo chariox && exit 0; exit 1; fi
if [ "\${1:-}" = "chariox" ]; then [ -f "$HARNESS_STATE/user" ]; exit $?; fi
exit 1
`)
  await writeHarnessCommand(join(bin, "getent"), `#!/bin/sh
if [ "\${1:-}" = "group" ] && [ "\${2:-}" = "chariox" ] && [ -f "$HARNESS_STATE/group" ]; then echo 'chariox:x:998:'; exit 0; fi
if [ "\${1:-}" = "passwd" ] && [ "\${2:-}" = "chariox" ] && [ -f "$HARNESS_STATE/user" ]; then echo 'chariox:x:998:998::/var/lib/chariox/home:/usr/sbin/nologin'; exit 0; fi
exit 2
`)
  await writeHarnessCommand(join(bin, "groupadd"), "#!/bin/sh\ntouch \"$HARNESS_STATE/group\"\n")
  await writeHarnessCommand(join(bin, "useradd"), "#!/bin/sh\ntouch \"$HARNESS_STATE/user\"\n")
  await writeHarnessCommand(join(bin, "systemctl"), "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HARNESS_STATE/systemctl\"\n")
  await writeHarnessCommand(join(bin, "find"), `#!/bin/sh
if [ "\${HARNESS_FIND_FAIL:-0}" = "1" ]; then exit 1; fi
exec /usr/bin/find "$@"
`)
  await writeHarnessCommand(join(bin, "install"), `#!/usr/bin/env node
const { spawnSync } = require("node:child_process")
const args = process.argv.slice(2)
const filtered = []
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === "-o" || args[index] === "-g") { index += 1; continue }
  filtered.push(args[index])
}
const result = spawnSync("/usr/bin/install", filtered, { stdio: "inherit" })
process.exit(result.status ?? 1)
`)
  return { bin, state, installRoot }
}

test("managed image installer verifies, installs twice, and rejects seeded runtime state", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
  }
  const args = [join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey]
  const badDigest = spawnSync(
    installer,
    [args[0], `sha256:${"0".repeat(64)}`, args[2]],
    { encoding: "utf8", env },
  )
  assert.equal(badDigest.status, 1)
  assert.match(badDigest.stderr, /release manifest digest does not match/)
  assert.equal(await lstat(harness.installRoot).then(() => true, () => false), false)
  const first = spawnSync(installer, args, { encoding: "utf8", env })
  const second = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(first.status, 0, first.stderr)
  assert.equal(second.status, 0, second.stderr)
  assert.equal(await readFile(join(harness.installRoot, "usr/local/bin/chariox-kernel"), "utf8"), "kernel fixture\n")
  assert.equal((await lstat(join(harness.installRoot, "usr/local/bin/chariox-kernel"))).mode & 0o777, 0o755)
  assert.equal((await lstat(join(harness.installRoot, "usr/lib/chariox/release-manifest.json"))).mode & 0o777, 0o644)
  assert.equal(
    await readFile(join(harness.installRoot, "etc/systemd/system/chariox-managed-bootstrap.service"), "utf8"),
    await readFile(service, "utf8"),
  )
  const systemctl = (await readFile(join(harness.state, "systemctl"), "utf8")).trim().split("\n")
  assert.deepEqual(systemctl, [
    "daemon-reload",
    "enable chariox-managed-bootstrap.service",
    "daemon-reload",
    "enable chariox-managed-bootstrap.service",
  ])

  const failedTraversal = spawnSync(installer, args, {
    encoding: "utf8",
    env: { ...env, HARNESS_FIND_FAIL: "1" },
  })
  assert.equal(failedTraversal.status, 1)
  assert.match(failedTraversal.stderr, /managed kernel state root could not be inspected/)

  const seededIdentity = join(harness.installRoot, "var/lib/chariox/home/daemon-machine-identity.json")
  await writeFile(seededIdentity, "should never enter an image")
  const rejected = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(rejected.status, 1)
  assert.match(rejected.stderr, /managed kernel state root is not pristine/)
  assert.equal(await readFile(seededIdentity, "utf8"), "should never enter an image")
})

test("managed image installer has no runtime start or network path", async () => {
  const contents = await readFile(installer, "utf8")
  assert.match(contents, /if \[ "\$\(id -u\)" -ne 0 \]/)
  assert.match(contents, /node "\$script_root\/verify-image-release\.mjs"/)
  assert.match(contents, /managed kernel state root is not pristine/)
  assert.match(contents, /groupadd --system chariox/)
  assert.match(contents, /useradd --system --gid chariox --home-dir \/var\/lib\/chariox\/home/)
  assert.match(contents, /systemctl daemon-reload/)
  assert.match(contents, /systemctl enable chariox-managed-bootstrap\.service/)
  assert.doesNotMatch(contents, /systemctl (?:start|restart|enable --now)/)
  assert.doesNotMatch(contents, /\b(?:curl|wget|ssh|scp)\b/)
  assert.doesNotMatch(contents, /\.arroba/)
})
