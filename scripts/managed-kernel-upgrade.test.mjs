import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { spawnSync } from "node:child_process"
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  rename,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { test } from "node:test"
import {
  rawPublicKey,
  makeRelease,
  put,
  createRootPrivateDirectory,
  makeHarness,
  installManagedHotpatchFacade,
  persistentSnapshot,
  treeSnapshot,
} from "./lib/managed-kernel-upgrade-fixture.mjs"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const upgrade = join(repositoryRoot, "deploy/managed-kernel/upgrade-image.sh")
const upgradeState = join(repositoryRoot, "deploy/managed-kernel/managed-kernel-upgrade-state.mjs")
const verifyRelease = join(repositoryRoot, "deploy/managed-kernel/verify-image-release.mjs")
const managedService = join(repositoryRoot, "deploy/managed-kernel/chariox-managed-bootstrap.service")
const serviceName = "chariox-managed-bootstrap.service"

// MP-07/MP-10: signed release ownership is a real root contract. Run the
// unchanged fixture assertions as root rather than simulating root-owned files.
if (process.platform === "linux" && process.getuid() !== 0) {
  test("managed upgrade root-owned release fixtures", () => {
    const result = spawnSync("sudo", ["--", process.execPath, "--test", fileURLToPath(import.meta.url)], {
      encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
      env: { ...process.env, NODE_TEST_CONTEXT: undefined },
    })
    process.stdout.write(result.stdout ?? "")
    assert.equal(result.error, undefined)
    assert.equal(result.signal, null)
    assert.equal(result.status, 0, result.stderr)
  })
} else {
test("managed kernel upgrade requires an explicit valid provider topology before reading the image", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-upgrade-topology-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const bin = join(root, "bin")
  await put(join(bin, "id"), "#!/bin/sh\nprintf '0\\n'\n", 0o755)
  const env = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    CHARIOX_MANAGED_UPGRADE_ROOT: join(root, "host"),
  }
  const args = ["missing-image", "wrong-current", "wrong-next", "missing-key"]
  for (const topology of [undefined, "", "unexpected"]) {
    const candidateEnv = { ...env }
    if (topology === undefined) delete candidateEnv.CHARIOX_MANAGED_PROVIDER_TOPOLOGY
    else candidateEnv.CHARIOX_MANAGED_PROVIDER_TOPOLOGY = topology
    const result = spawnSync(upgrade, args, { encoding: "utf8", env: candidateEnv })
    assert.equal(result.status, 1)
    assert.match(result.stderr, topology === "unexpected"
      ? /CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be path1 or shared_host/
      : /CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be explicitly set to path1 or shared_host/)
    assert.doesNotMatch(result.stderr, /release digest|image root/)
  }
  for (const topology of ["path1", "shared_host"]) {
    const result = spawnSync(upgrade, args, {
      encoding: "utf8",
      env: { ...env, CHARIOX_MANAGED_PROVIDER_TOPOLOGY: topology },
    })
    assert.equal(result.status, 1)
    assert.match(result.stderr, /managed release digest is invalid/)
  }
})

test("Path-1 upgrade drop-in guard checks reload freshness and both effective services", async (context) => {
  const source = await readFile(upgrade, "utf8")
  const guard = source.match(/assert_path1_service_overrides\(\) \{\n[\s\S]*?^\}/m)?.[0]
  assert.ok(guard)
  assert.match(guard, /systemctl show --property=NeedDaemonReload --value "\$unit"/)
  assert.match(guard, /systemctl show --property=DropInPaths --value "\$unit"/)
  assert.ok(guard.indexOf("--property=NeedDaemonReload") < guard.indexOf("--property=DropInPaths"))
  assert.match(source, /select_supervisor_service\nassert_path1_service_overrides\nif \[ "\$recover_only" -eq 1 \]; then/)
  assert.ok(source.indexOf("\nassert_path1_service_overrides\n") < source.indexOf("\nrecover_transaction\n"))
  assert.match(source, /systemctl daemon-reload \|\| return 1\n  assert_path1_service_overrides \|\| return 1\n  start_path1_runtime_services \|\| return 1\n  start_managed_app_storage \|\| return 1\n  health_not_before_ms=/)
  assert.match(source, /if ! systemctl daemon-reload \\\n  \|\| ! assert_path1_service_overrides \\\n  \|\| ! start_path1_runtime_services \\\n/)

  const scratch = await mkdtemp(join(tmpdir(), "chariox-upgrade-dropin-"))
  context.after(() => rm(scratch, { recursive: true, force: true }))
  const bin = join(scratch, "bin")
  await put(join(bin, "systemctl"), `#!/bin/sh
case "$*" in
  *--property=NeedDaemonReload*chariox-path1-managed-bootstrap.service) printf '%s' "\${SYSTEMD_HOME_NEED_DAEMON_RELOAD:-no}" ;;
  *--property=NeedDaemonReload*chariox-disposable-worker-bootstrap.service) printf '%s' "\${SYSTEMD_WORKER_NEED_DAEMON_RELOAD:-no}" ;;
  *--property=DropInPaths*chariox-path1-managed-bootstrap.service) printf '%s' "\${SYSTEMD_HOME_DROP_IN_PATHS:-}" ;;
  *--property=DropInPaths*chariox-disposable-worker-bootstrap.service) printf '%s' "\${SYSTEMD_WORKER_DROP_IN_PATHS:-}" ;;
esac
`, 0o755)
  const command = `managed_provider_topology=path1\n${guard}\nassert_path1_service_overrides\n`
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}` }
  const clean = spawnSync("/bin/sh", ["-c", command], { encoding: "utf8", env })
  assert.equal(clean.status, 0, clean.stderr)
  for (const [name, unit] of [
    ["SYSTEMD_HOME_DROP_IN_PATHS", "chariox-path1-managed-bootstrap.service"],
    ["SYSTEMD_WORKER_DROP_IN_PATHS", "chariox-disposable-worker-bootstrap.service"],
  ]) {
    const inherited = spawnSync("/bin/sh", ["-c", command], {
      encoding: "utf8",
      env: { ...env, [name]: "/etc/systemd/system/50-hardening.conf" },
    })
    assert.equal(inherited.status, 1)
    assert.match(inherited.stderr, new RegExp(`Path-1 service ${unit.replaceAll(".", "\\.")} has systemd drop-ins`))
  }
  for (const [name, unit] of [
    ["SYSTEMD_HOME_NEED_DAEMON_RELOAD", "chariox-path1-managed-bootstrap.service"],
    ["SYSTEMD_WORKER_NEED_DAEMON_RELOAD", "chariox-disposable-worker-bootstrap.service"],
  ]) {
    const stale = spawnSync("/bin/sh", ["-c", command], {
      encoding: "utf8",
      env: { ...env, [name]: "yes" },
    })
    assert.equal(stale.status, 1)
    assert.match(stale.stderr, new RegExp(`Path-1 service ${unit.replaceAll(".", "\\.")} needs systemd daemon-reload`))
  }
})

test("repository release policy admits reviewed predecessors and matches the runtime protocol", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-release-policy-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const current = join(root, "current")
  const target = join(root, "target")
  const policyPath = "usr/lib/chariox/slice-build-context/apps/kernel/managed-upgrade-protocol-transitions.json"
  await mkdir(current)
  const policyBytes = await readFile(join(repositoryRoot, "apps/kernel/managed-upgrade-protocol-transitions.json"))
  const policy = JSON.parse(policyBytes)
  const runtimeTypes = await readFile(join(repositoryRoot, "apps/kernel/src/local/api/types.rs"), "utf8")
  const runtimeProtocol = Number(runtimeTypes.match(/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+);/)[1])
  // MP-07/MP-10: released F/G/Apps predecessors and the current union; unreleased379..409 remain refused.
  const admittedProtocols = [343, ...Array.from({ length: 12 }, (_, index) => 367 + index), 410, 411, 415, 416, 435, runtimeProtocol]
  assert.deepEqual(policy, {
    schemaVersion: 1,
    protocol: runtimeProtocol,
    upgradeFrom: admittedProtocols,
    rollbackTo: admittedProtocols,
  })
  await put(join(current, policyPath), policyBytes)
  await put(join(target, policyPath), policyBytes)

  for (const olderProtocol of admittedProtocols.filter(version => version !== runtimeProtocol)) {
    for (const [currentRoot, currentProtocol, targetRoot, targetProtocol] of [
      [current, olderProtocol, target, runtimeProtocol],
      [target, runtimeProtocol, current, olderProtocol],
    ]) {
      const result = spawnSync(process.execPath, [upgradeState, "validate-protocol-transition",
        currentRoot, String(currentProtocol), targetRoot, String(targetProtocol)], { encoding: "utf8" })
      assert.equal(result.status, 0, result.stderr)
    }
  }

  for (const unsupportedProtocol of [312, 325, 333, 339, 342, ...Array.from({ length: 23 }, (_, index) => 344 + index), ...Array.from({ length: 31 }, (_, index) => 379 + index)]) {
    for (const [currentRoot, currentProtocol, targetRoot, targetProtocol] of [
      [current, unsupportedProtocol, target, runtimeProtocol],
      [target, runtimeProtocol, current, unsupportedProtocol],
    ]) {
      const result = spawnSync(process.execPath, [upgradeState, "validate-protocol-transition",
        currentRoot, String(currentProtocol), targetRoot, String(targetProtocol)], { encoding: "utf8" })
      assert.equal(result.status, 1, `${currentProtocol} to ${targetProtocol} should be denied`)
      assert.match(result.stderr, /not reciprocally authorized/)
    }
  }
})

test("Path-1 upgrade fixture has a verified builder attestation and both supervisor units", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-path1-upgrade-fixture-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const releaseKeys = generateKeyPairSync("ed25519")
  const builderKeys = generateKeyPairSync("ed25519")
  const releaseKey = join(root, "release-public-key")
  const builderKey = join(root, "builder-public-key")
  await put(releaseKey, rawPublicKey(releaseKeys.publicKey).toString("base64"))
  await put(builderKey, rawPublicKey(builderKeys.publicKey).toString("base64"))
  const release = await makeRelease(
    root, "path1", 325, releaseKeys.privateKey, releaseKeys.publicKey, null, true, builderKeys,
  )
  const result = spawnSync(process.execPath, [
    verifyRelease, release.rootfs, release.digest, releaseKey, "path1", builderKey,
  ], { encoding: "utf8" })
  assert.equal(result.status, 0, result.stderr)
})

test("managed kernel upgrade atomically advances the release and receipt without changing persistent state", async (context) => {
  const harness = await makeHarness(context)
  const before = await persistentSnapshot(harness.persistent)
  const receiptOwner = await stat(harness.receiptPath)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  const receipt = JSON.parse(await readFile(harness.receiptPath, "utf8"))
  assert.deepEqual(receipt, { ...harness.receipt, runtimeReleaseDigest: harness.target.digest })
  const upgradedReceiptOwner = await stat(harness.receiptPath)
  assert.equal(upgradedReceiptOwner.uid, receiptOwner.uid)
  assert.equal(upgradedReceiptOwner.gid, receiptOwner.gid)
  assert.equal(upgradedReceiptOwner.mode & 0o777, receiptOwner.mode & 0o777)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  assert.equal(
    await readFile(join(harness.installRoot, "usr/lib/chariox/current/usr/local/bin/chariox-managed-bootstrap"), "utf8"),
    "#!/bin/sh\necho supervisor-target\n",
  )
  assert.deepEqual((await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n"), [
    `stop ${serviceName}`,
    "daemon-reload",
    "enable chariox-app-storage.service",
    "restart chariox-app-storage.service",
    "is-active --quiet chariox-app-storage.service",
    `start ${serviceName}`,
    `is-active --quiet ${serviceName}`,
  ])
})

test("managed kernel upgrade stages the App artifact set when advancing from a release built before Apps", async (context) => {
  const harness = await makeHarness(context, { currentAppArtifacts: false })
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  const targetRelease = join(harness.installRoot, `usr/lib/chariox/releases/${harness.target.digest.slice("sha256:".length)}`)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  for (const [path, contents, mode] of [
    ["usr/local/bin/chariox-app-package", "#!/bin/sh\necho app-package-target\n", 0o755],
    ["usr/libexec/chariox-app-storage", "#!/bin/sh\necho app-storage-target\n", 0o755],
    ["etc/systemd/system/chariox-app-storage.service",
      await readFile(join(repositoryRoot, "deploy/managed-kernel/chariox-app-storage.service"), "utf8"), 0o644],
  ]) {
    assert.equal(await readFile(join(targetRelease, path), "utf8"), contents)
    assert.equal((await stat(join(targetRelease, path))).mode & 0o777, mode)
  }
  const config = JSON.parse(await readFile(join(harness.installRoot, "etc/chariox/app-storage.json"), "utf8"))
  assert.deepEqual(config.owners, [{uid: harness.charioxIdentity.uid, gid: harness.charioxIdentity.gid,
    cgroup_root: "/sys/fs/cgroup/system.slice/chariox-managed-bootstrap.service/apps",
    kernel_database_paths: ["/home/chariox/.chariox/state/kernel.db"]}])
  for (const path of ["usr/local/bin/chariox-app-package", "usr/libexec/chariox-app-storage",
    "etc/systemd/system/chariox-app-storage.service"]) {
    assert.equal(await readFile(join(harness.installRoot, path), "utf8"), await readFile(join(targetRelease, path), "utf8"))
  }
  const serviceLog = await readFile(join(harness.state, "systemctl.log"), "utf8")
  assert.match(serviceLog, /enable chariox-app-storage.service/)
  assert.ok(serviceLog.indexOf("restart chariox-app-storage.service") < serviceLog.indexOf("start chariox-managed-bootstrap.service"))
  const rollback = harness.run({}, [harness.current.rootfs, harness.target.digest, harness.current.digest, harness.trustedKey])
  assert.equal(rollback.status, 0, rollback.stderr)
  assert.match(rollback.stderr, /App storage is disabled/)
  for (const path of ["usr/local/bin/chariox-app-package", "usr/libexec/chariox-app-storage",
    "etc/systemd/system/chariox-app-storage.service"]) {
    assert.equal(await lstat(join(harness.installRoot, path)).then(() => true, () => false), false)
  }
  assert.deepEqual(JSON.parse(await readFile(join(harness.installRoot, "etc/chariox/app-storage.json"), "utf8")), config)

  const previousRelease = join(harness.installRoot, `usr/lib/chariox/releases/${harness.current.digest.slice("sha256:".length)}`)
  assert.equal(await lstat(join(previousRelease, "usr/local/bin/chariox-app-package")).then(() => true, () => false), false)
})

test("failed release F upgrade disables App links while preserving storage enrollment", async (context) => {
  const harness = await makeHarness(context, {currentAppArtifacts: false})
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /restored previous managed kernel release/)
  assert.match(result.stderr, /App storage is disabled/)
  for (const path of ["usr/local/bin/chariox-app-package", "usr/libexec/chariox-app-storage",
    "etc/systemd/system/chariox-app-storage.service"]) {
    assert.equal(await lstat(join(harness.installRoot, path)).then(() => true, () => false), false)
  }
  assert.match(await readFile(join(harness.state, "systemctl.log"), "utf8"), /disable --now chariox-app-storage.service/)
  assert.equal(await lstat(join(harness.installRoot, "etc/chariox/app-storage.json")).then(() => true, () => false), true)
})

test("failed pre-Apps rollback resumes after current changed but before App links were removed", async context => {
  const harness = await makeHarness(context, {currentAppArtifacts: false})
  await put(join(harness.state, "fail-health-once"), "fail\n")
  await put(join(harness.state, "crash-after-pre-apps-current"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const unit = join(harness.installRoot, "etc/systemd/system/chariox-app-storage.service")
  assert.equal((await lstat(unit)).isSymbolicLink(), true)
  assert.equal(await stat(unit).then(() => true, () => false), false, "current now makes the unit dangle")
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), `releases/${harness.current.digest.slice(7)}`)
  // Re-run the same command. It recovers the interrupted rollback first, then
  // makes a fresh successful upgrade; no operator repairs the dangling link.
  const recovered = harness.run()
  assert.equal(recovered.status, 0, recovered.stderr)
  assert.match(recovered.stderr, /App storage is disabled/)
  assert.equal((await stat(unit)).isFile(), true)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).split("\n")
  assert.equal(calls.filter(line => line === "disable --now chariox-app-storage.service").length, 1)
})

test("Path-1 upgrade rejects effective home or worker drop-ins before recovery or service mutation", async (context) => {
  for (const [marker, unit] of [
    ["home-drop-in", "chariox-path1-managed-bootstrap.service"],
    ["worker-drop-in", "chariox-disposable-worker-bootstrap.service"],
  ]) {
    const harness = await makeHarness(context)
    await put(join(harness.state, marker), "present\n")
    const priorReceipt = await readFile(harness.receiptPath, "utf8")
    const priorRelease = await readlink(join(harness.installRoot, "usr/lib/chariox/current"))
    const result = harness.run({
      CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
      CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedKey,
    })
    assert.equal(result.status, 1)
    assert.match(result.stderr, new RegExp(`Path-1 service ${unit.replaceAll(".", "\\.")} has systemd drop-ins`))
    const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
    assert.ok(calls.every((call) => call.startsWith("show --property=")), calls.join("\n"))
    assert.equal(await readFile(harness.receiptPath, "utf8"), priorReceipt)
    assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), priorRelease)
  }
})

test("Path-1 upgrade refuses an unreloaded on-disk drop-in before pending transaction recovery or service mutation", async (context) => {
  const services = [
    ["disk-home-drop-in", "chariox-path1-managed-bootstrap.service"],
    ["disk-worker-drop-in", "chariox-disposable-worker-bootstrap.service"],
  ]
  for (const phase of ["prepared", "stopped"]) {
    for (const [diskDropIn, blockedUnit] of services) {
      const harness = await makeHarness(context, { path1Release: true })
      await put(join(harness.state, `crash-after-phase-${phase}`), "crash\n")
      const seeded = harness.run({
        CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
        CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
      })
      assert.equal(seeded.signal, "SIGKILL", `${phase}: ${seeded.stderr}`)

      const transactionRoot = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade")
      assert.equal(await readFile(join(transactionRoot, "phase"), "utf8"), `${phase}\n`)
      const beforeInstall = await treeSnapshot(harness.installRoot)
      const beforeTransaction = await treeSnapshot(transactionRoot)
      const beforeSystemctl = (await readFile(join(harness.state, "systemctl.log"), "utf8"))
        .trim().split("\n").length
      await put(join(harness.state, diskDropIn), "unreloaded systemd drop-in\n")

      const result = harness.run({
        CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
        CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
      })
      assert.equal(result.status, 1, `${phase}/${blockedUnit}: ${result.stderr}`)
      assert.match(result.stderr, new RegExp(
        `Path-1 service ${blockedUnit.replaceAll(".", "\\.")} needs systemd daemon-reload; refusing upgrade before service mutation`,
      ))

      const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8"))
        .trim().split("\n").slice(beforeSystemctl)
      assert.ok(calls.every((call) => call.startsWith("show --property=")), calls.join("\n"))
      assert.ok(calls.includes("show --property=NeedDaemonReload --value chariox-path1-managed-bootstrap.service"), calls.join("\n"))
      assert.ok(calls.includes("show --property=NeedDaemonReload --value chariox-disposable-worker-bootstrap.service"), calls.join("\n"))
      assert.ok(calls.includes("show --property=DropInPaths --value chariox-path1-managed-bootstrap.service"), calls.join("\n"))
      assert.ok(calls.includes("show --property=DropInPaths --value chariox-disposable-worker-bootstrap.service"), calls.join("\n"))
      assert.deepEqual(await treeSnapshot(transactionRoot), beforeTransaction)
      assert.deepEqual(await treeSnapshot(harness.installRoot), beforeInstall)
    }
  }
})

test("MP-07/MP-10/MP-11 installed diagnostics permit upgrade but an unrelated override still blocks it", async (context) => {
  for (const unrelated of [false, true]) {
    const harness = await makeHarness(context, { path1Release: true })
    let installer = await readFile(join(repositoryRoot, "deploy/managed-kernel/enable-campaign-diagnostics.sh"), "utf8")
    for (const path of ["/etc/systemd/system", "/home/chariox/.chariox", "/usr/lib/chariox"]) {
      installer = installer.replaceAll(path, `${harness.installRoot}${path}`)
    }
    installer = installer.replace(`Environment=CHARIOX_RUNTIME_DIAGNOSTICS_DIR=${harness.installRoot}`, "Environment=CHARIOX_RUNTIME_DIAGNOSTICS_DIR=")
    installer = installer.replace("-o chariox -g chariox", `-o ${harness.charioxIdentity.uid} -g ${harness.charioxIdentity.gid}`)
    // Root-mapped installer; only the fixture systemctl activity check differs.
    installer = installer.replace("systemctl is-active --quiet", "false is-active --quiet")
    const installed = spawnSync("/bin/sh", ["-s", "--", "https://observer.example/path1-diagnostics/round-20261009c"], {
      input: installer, encoding: "utf8", env: harness.env,
    })
    assert.equal(installed.status, 0, installed.stderr)
    const config = join(harness.installRoot, "etc/systemd/system/chariox-path1-managed-bootstrap.service.d/path1-campaign-diagnostics.conf")
    await put(join(harness.state, "campaign-drop-in-path"), `${config}\n`)
    if (unrelated) await put(join(harness.state, "home-drop-in"), "present\n")
    const result = harness.run({
      CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
      CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
    })
    assert.equal(result.status, unrelated ? 1 : 0, result.stderr)
    const calls = await readFile(join(harness.state, "systemctl.log"), "utf8")
    if (unrelated) {
      assert.match(result.stderr, /has systemd drop-ins/)
      assert.doesNotMatch(calls, /stop chariox-path1-managed-bootstrap.service/)
    } else {
      assert.match(calls, /stop chariox-path1-managed-bootstrap.service/)
      assert.match(calls, /start chariox-path1-managed-bootstrap.service/)
      assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
        `releases/${harness.target.digest.slice("sha256:".length)}`)
    }
  }
})

test("Path-1 upgrade starts the selected supervisor after both effective units pass the drop-in check", async (context) => {
  const harness = await makeHarness(context, { path1Release: true })
  const result = harness.run({
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
  })
  assert.equal(result.status, 0, result.stderr)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.ok(calls.includes("show --property=DropInPaths --value chariox-path1-managed-bootstrap.service"), calls.join("\n"))
  assert.ok(calls.includes("show --property=DropInPaths --value chariox-disposable-worker-bootstrap.service"), calls.join("\n"))
  assert.ok(calls.includes("daemon-reload"), calls.join("\n"))
  const findCall = (call) => calls.indexOf(call)
  assert.ok(findCall("stop chariox-path1-managed-bootstrap.service") < findCall("stop chariox-rootless-docker.service"), calls.join("\n"))
  assert.ok(findCall("stop chariox-rootless-docker.service") < findCall("stop user@1001.service"), calls.join("\n"))
  assert.ok(findCall("stop user@1001.service") < findCall("stop chariox-slice-disk-quota-allocator.service"), calls.join("\n"))
  assert.ok(findCall("stop chariox-slice-disk-quota-allocator.service") < findCall("stop chariox-data-volume-admission.service"), calls.join("\n"))
  assert.ok(findCall("daemon-reload") < findCall("start chariox-rootless-docker.service"), calls.join("\n"))
  assert.ok(findCall("start chariox-rootless-docker.service") < findCall("start chariox-path1-managed-bootstrap.service"), calls.join("\n"))
  assert.ok(calls.includes("start chariox-slice-disk-quota-allocator.service"), calls.join("\n"))
  assert.ok(calls.includes("start chariox-rootless-docker.service"), calls.join("\n"))
  assert.ok(calls.includes("is-active --quiet chariox-slice-disk-quota-allocator.service"), calls.join("\n"))
  assert.ok(calls.includes("is-active --quiet chariox-rootless-docker.service"), calls.join("\n"))
  assert.ok(calls.includes("is-active --quiet user@1001.service"), calls.join("\n"))
  assert.equal((await readFile(join(harness.state, "mountpoint.log"), "utf8")).trim(), "--quiet /var/lib/chariox-docker/data")
  assert.ok(calls.includes("start chariox-path1-managed-bootstrap.service"), calls.join("\n"))
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`)
})

test("Path-1 upgrade leaves the kernel stopped and rollback pending when a worker drop-in appears after reload", async (context) => {
  const harness = await makeHarness(context, { path1Release: true })
  const priorReceipt = await readFile(harness.receiptPath, "utf8")
  const priorRelease = await readlink(join(harness.installRoot, "usr/lib/chariox/current"))
  await put(join(harness.state, "worker-drop-in-after-reload"), "present\n")
  const result = harness.run({
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
  })
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /Path-1 service chariox-disposable-worker-bootstrap\.service has systemd drop-ins/)
  assert.match(result.stderr, /Path-1 systemd drop-ins blocked activation; rollback remains pending; verify the managed kernel service state before retry/)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.ok(calls.includes("daemon-reload"), calls.join("\n"))
  assert.ok(calls.includes("stop chariox-path1-managed-bootstrap.service"), calls.join("\n"))
  assert.ok(calls.every((call) => !call.startsWith("start ")), calls.join("\n"))
  assert.equal(await readFile(harness.receiptPath, "utf8"), priorReceipt)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), priorRelease)
  assert.equal((await stat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))).isDirectory(), true)
})

test("managed kernel upgrade migrates legacy home state and rejects a home collision", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)

  const migrated = harness.run()
  assert.equal(migrated.status, 0, migrated.stderr)
  assert.equal(await lstat(legacyHome).then(() => true, () => false), false)
  assert.equal(
    await readFile(join(harness.installRoot, "home/chariox/repositories/repo-1/HEAD"), "utf8"),
    "repository-sentinel\n",
  )
  assert.equal(
    await readFile(join(harness.installRoot, "home/chariox/.chariox/vault/vault.json"), "utf8"),
    "credential-sentinel\n",
  )
  assert.equal(
    JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest,
    harness.target.digest,
  )
  assert.equal((await stat(join(harness.installRoot, "home/chariox"))).mode & 0o777, 0o700)
  assert.equal((await stat(join(harness.installRoot, "home/chariox/.chariox"))).mode & 0o777, 0o700)
  const { uid: charioxUid, gid: charioxGid } = harness.charioxIdentity
  assert.equal((await stat(join(harness.installRoot, "home/chariox"))).uid, charioxUid)
  assert.equal((await stat(join(harness.installRoot, "home/chariox"))).gid, charioxGid)
  assert.equal((await stat(join(harness.installRoot, "home/chariox/.chariox"))).uid, charioxUid)
  assert.equal((await stat(join(harness.installRoot, "home/chariox/.chariox"))).gid, charioxGid)

  // The empty skeleton older Cloud cloud-init created holds no state and is removed.
  const skeletonHarness = await makeHarness(context)
  const skeletonLegacy = join(skeletonHarness.installRoot, "var/lib/chariox/home")
  await createRootPrivateDirectory(skeletonLegacy)
  await createRootPrivateDirectory(join(skeletonLegacy, "managed"))
  const skeleton = skeletonHarness.run()
  assert.equal(skeleton.status, 0, skeleton.stderr)
  assert.equal(await lstat(skeletonLegacy).then(() => true, () => false), false)

  // One nested file makes the skeleton a real legacy home: still a collision.
  const nestedHarness = await makeHarness(context)
  const nestedLegacy = join(nestedHarness.installRoot, "var/lib/chariox/home")
  await createRootPrivateDirectory(nestedLegacy)
  await createRootPrivateDirectory(join(nestedLegacy, "managed"))
  await writeFile(join(nestedLegacy, "managed", "state"), "keep\n")
  const nested = nestedHarness.run()
  assert.equal(nested.status, 1)
  assert.match(nested.stderr, /both exist; refusing to overwrite either/)
  assert.equal(await readFile(join(nestedLegacy, "managed", "state"), "utf8"), "keep\n")

  const collisionHarness = await makeHarness(context)
  const collisionLegacy = join(collisionHarness.installRoot, "var/lib/chariox/home")
  await createRootPrivateDirectory(collisionLegacy)
  await writeFile(join(collisionLegacy, "must-remain-untouched"), "collision\n")
  const collision = collisionHarness.run()
  assert.equal(collision.status, 1)
  assert.match(collision.stderr, /both exist; refusing to overwrite either/)
  assert.equal(
    await readFile(join(collisionLegacy, "must-remain-untouched"), "utf8"),
    "collision\n",
  )
  assert.equal(
    await lstat(join(collisionHarness.state, "systemctl.log")).then(() => true, () => false),
    false,
  )
})

test("managed kernel upgrade stops the live service before migrating legacy home", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)
  await put(join(harness.state, "write-legacy-home-on-stop"), "write\n")

  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await lstat(legacyHome).then(() => true, () => false),
    false,
    "the stopped service must not recreate a legacy state tree after migration",
  )
  assert.equal(
    await readFile(join(canonicalHome, "late-service-write"), "utf8"),
    "late-service-write\n",
    "the migration must retain the service's final write",
  )
  assert.deepEqual((await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n"), [
    `stop ${serviceName}`,
    "daemon-reload",
    "enable chariox-app-storage.service",
    "restart chariox-app-storage.service",
    "is-active --quiet chariox-app-storage.service",
    `start ${serviceName}`,
    `is-active --quiet ${serviceName}`,
  ])
})

test("managed kernel upgrade resumes its identity-backed home migration after crash windows", async (context) => {
  const crashWindows = ["crash-after-home-root-rename", "crash-before-home-completion-marker"]
  const observedCrashWindows = []
  for (const marker of crashWindows) {
    const harness = await makeHarness(context)
    const canonicalHome = join(harness.installRoot, "home/chariox")
    const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
    const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
    await rename(canonicalHome, legacyHome)
    await createRootPrivateDirectory(dirname(legacyReceipt))
    await rename(harness.receiptPath, legacyReceipt)
    await put(join(harness.state, marker), "crash\n")

    const interrupted = harness.run()
    assert.equal(interrupted.signal, "SIGKILL", `${marker}: ${interrupted.stderr}`)
    observedCrashWindows.push(marker)
    const recovered = harness.run()
    assert.equal(recovered.status, 0, `${marker}: ${recovered.stderr}`)
    assert.equal(await lstat(legacyHome).then(() => true, () => false), false)
    assert.equal(
      await readFile(join(canonicalHome, "repositories/repo-1/HEAD"), "utf8"),
      "repository-sentinel\n",
    )
    assert.equal(
      JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest,
      harness.target.digest,
    )
  }
  assert.deepEqual(observedCrashWindows, crashWindows, "both injected home-migration crash windows must be observed")
})

test("managed kernel upgrade validates the signed candidate before planning legacy migration", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)
  await put(
    join(harness.target.rootfs, "etc/systemd/system/chariox-disposable-worker-bootstrap.service"),
    "[Service]\nExecStart=/unexpected\n",
  )

  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /chariox-disposable-worker-bootstrap.service is corrupted/)
  assert.equal(await lstat(legacyHome).then(() => true, () => false), true)
  assert.equal(await lstat(canonicalHome).then(() => true, () => false), false)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects a symlinked legacy state entry before stopping the service", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  const outside = join(harness.root, "outside-managed-context")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)
  await mkdir(outside)
  await writeFile(join(outside, "must-remain"), "outside\n")
  await symlink(outside, join(legacyHome, "managed-context"))

  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /managed kernel managed-context is not a real directory/)
  assert.equal(await readFile(join(outside, "must-remain"), "utf8"), "outside\n")
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed upgrade rejects a corrupted declared worker service before stopping the kernel", async (context) => {
  const harness = await makeHarness(context)
  await put(
    join(harness.target.rootfs, "etc/systemd/system/chariox-disposable-worker-bootstrap.service"),
    "[Service]\nExecStart=/unexpected\n",
  )
  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /chariox-disposable-worker-bootstrap.service is corrupted/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("legacy managed release rejects an unsigned extra worker service before stopping the kernel", async (context) => {
  const harness = await makeHarness(context)
  await put(
    join(harness.installRoot, "usr/lib/chariox/current/etc/systemd/system/chariox-disposable-worker-bootstrap.service"),
    "[Service]\nExecStart=/unexpected\n",
  )
  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /undeclared worker service/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

for (const failHealth of [false, true]) test(`Path-1 builder rotation ${failHealth ? "restores the previous pin on rollback" : "activates the next independent pin"}`, async (context) => {
  const harness = await makeHarness(context, { path1Release: true, rotateBuilder: true })
  const runtimePin = join(harness.installRoot, "etc/chariox/trusted-builder-public-key")
  const previousPin = await readFile(harness.trustedBuilderKey, "utf8")
  const nextPin = await readFile(harness.nextTrustedBuilderKey, "utf8")
  await put(runtimePin, previousPin, 0o644)
  await put(join(harness.state, "check-builder-pin-on-start"), "check\n")
  if (failHealth) await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run({
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
    CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: harness.nextTrustedBuilderKey,
  })
  assert.equal(result.status, failHealth ? 1 : 0, result.stderr)
  if (failHealth) assert.match(result.stderr, /health check failed; restored previous/)
  assert.equal(await readFile(runtimePin, "utf8"), failHealth ? previousPin : nextPin)
  assert.equal((await stat(runtimePin)).mode & 0o777, 0o644)
  assert.equal((await readFile(join(harness.state, "builder-pin-starts"), "utf8")).trim().split("\n").length, failHealth ? 2 : 1)
  const selected = failHealth ? harness.current : harness.target
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), `releases/${selected.digest.slice(7)}`)
})

test("Path-1 builder rotation supports simultaneous release-signing key rotation", async (context) => {
  const harness = await makeHarness(context, { path1Release: true, rotateBuilder: true })
  const keys = generateKeyPairSync("ed25519")
  const nextReleasePin = join(harness.root, "next-release-pin")
  const manifest = await readFile(join(harness.target.rootfs, "usr/lib/chariox/release-manifest.json"))
  await put(join(harness.target.rootfs, "usr/lib/chariox/release-manifest.sig"), sign(null, manifest, keys.privateKey).toString("base64"))
  await put(join(harness.target.rootfs, "usr/lib/chariox/release-public-key"), rawPublicKey(keys.publicKey).toString("base64"))
  await put(nextReleasePin, rawPublicKey(keys.publicKey).toString("base64"), 0o600)
  const runtimePin = join(harness.installRoot, "etc/chariox/trusted-builder-public-key")
  await put(runtimePin, await readFile(harness.trustedBuilderKey), 0o644)
  await put(join(harness.state, "check-builder-pin-on-start"), "check\n")
  const result = harness.run({
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
    CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: harness.nextTrustedBuilderKey,
  }, [harness.target.rootfs, harness.current.digest, harness.target.digest, harness.trustedKey, nextReleasePin])
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readFile(runtimePin, "utf8"), await readFile(harness.nextTrustedBuilderKey, "utf8"))
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), `releases/${harness.target.digest.slice(7)}`)
})

test("MP-07 Path-1 builder rotation restores a killed pin helper before a fresh upgrade", async (context) => {
  const harness = await makeHarness(context, { path1Release: true, rotateBuilder: true })
  const runtimePin = join(harness.installRoot, "etc/chariox/trusted-builder-public-key")
  await put(runtimePin, await readFile(harness.trustedBuilderKey), 0o644)
  await put(join(harness.state, "check-builder-pin-on-start"), "check\n")
  await put(join(harness.state, "crash-after-builder-pin"), "crash\n")
  const before = await persistentSnapshot(harness.persistent)
  const env = {
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
    CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: harness.nextTrustedBuilderKey,
  }
  const crashed = harness.run(env)
  assert.equal(crashed.status, 1, crashed.stderr)
  assert.equal(await readFile(runtimePin, "utf8"), await readFile(harness.trustedBuilderKey, "utf8"))
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), `releases/${harness.current.digest.slice(7)}`)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  assert.equal(await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade")).then(() => true, () => false), false)
  const recovered = harness.run(env)
  assert.equal(recovered.status, 0, recovered.stderr)
  assert.equal(await readFile(runtimePin, "utf8"), await readFile(harness.nextTrustedBuilderKey, "utf8"))
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  assert.equal((await readFile(join(harness.state, "builder-pin-starts"), "utf8")).trim().split("\n").length, 2)
  assert.equal(await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade")).then(() => true, () => false), false)
})

for (const wrongPin of ["current", "next"]) test(`Path-1 builder rotation rejects a wrong ${wrongPin} pin before stopping services`, async (context) => {
  const harness = await makeHarness(context, { path1Release: true, rotateBuilder: true })
  const runtimePin = join(harness.installRoot, "etc/chariox/trusted-builder-public-key")
  const previousPin = await readFile(harness.trustedBuilderKey, "utf8")
  await put(runtimePin, previousPin, 0o644)
  const result = harness.run({
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: wrongPin === "current" ? harness.nextTrustedBuilderKey : harness.trustedBuilderKey,
    CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY: wrongPin === "next" ? harness.trustedBuilderKey : harness.nextTrustedBuilderKey,
  })
  assert.notEqual(result.status, 0)
  const calls = await readFile(join(harness.state, "systemctl.log"), "utf8").catch(() => "")
  assert.doesNotMatch(calls, /^stop /m)
  assert.equal(await readFile(runtimePin, "utf8"), previousPin)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), `releases/${harness.current.digest.slice(7)}`)
})

test("managed kernel upgrade rotates the release trust key explicitly", async (context) => {
  const harness = await makeHarness(context)
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const targetManifest = await readFile(join(harness.target.rootfs, "usr/lib/chariox/release-manifest.json"))
  await put(
    join(harness.target.rootfs, "usr/lib/chariox/release-manifest.sig"),
    sign(null, targetManifest, privateKey).toString("base64"),
  )
  const nextTrustedKey = join(harness.root, "next-trusted-release-public-key")
  const nextRawKey = rawPublicKey(publicKey).toString("base64")
  await put(join(harness.target.rootfs, "usr/lib/chariox/release-public-key"), nextRawKey)
  await put(nextTrustedKey, nextRawKey, 0o600)

  const result = harness.run({}, [
    harness.target.rootfs,
    harness.current.digest,
    harness.target.digest,
    harness.trustedKey,
    nextTrustedKey,
  ])
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel upgrade supersedes a managed-hotpatch slice context facade", async (context) => {
  const harness = await makeHarness(context)
  const { facade } = await installManagedHotpatchFacade(harness)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readlink(facade), "current/usr/lib/chariox/slice-build-context")
  assert.equal(
    await readFile(join(facade, "apps/kernel/slice-linux-docker/runtime.txt"), "utf8"),
    "target\n",
  )
})

test("managed kernel failed health restores the exact prior slice context facade", async (context) => {
  const harness = await makeHarness(context)
  const { facade, hotpatch } = await installManagedHotpatchFacade(harness)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(await readlink(facade), hotpatch)
})

test("managed kernel recovery restores a hotpatch facade after interrupted activation", async (context) => {
  const harness = await makeHarness(context)
  const { facade, hotpatch } = await installManagedHotpatchFacade(harness)
  await put(join(harness.state, "crash-after-facade-symlink"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  assert.equal(await readlink(facade), "current/usr/lib/chariox/slice-build-context")

  await put(join(harness.target.rootfs, "usr/local/bin/chariox-kernel"), "#!/bin/sh\nexit 1\n", 0o755)
  const recovered = harness.run()
  assert.equal(recovered.status, 1)
  assert.match(recovered.stderr, /release artifact chariox-kernel is corrupted/)
  assert.equal(await readlink(facade), hotpatch)
})

test("allocation worker upgrade preserves the receipt written by the active bootstrap", async (context) => {
  const harness = await makeHarness(context, { workerCapableCurrent: true })
  // Keep this shape aligned with WorkerReceipt in managed_bootstrap/worker.rs,
  // not the disabled pre-allocation bootstrap's binding/enrollmentReceipt shape.
  const receipt = {
    schemaVersion: 1,
    status: "confirmed",
    allocationId: "allocation-1",
    machineId: "machine-1",
    kernelId: "kernel-1",
    relayPublicKey: "relay-public-key",
    runtimeReleaseDigest: harness.current.digest,
    homeCaller: {
      accountId: "account-1", userId: "owner-1", realmId: "realm-1",
      machineId: "home-machine-1", kernelId: "home-kernel-1", relayPublicKey: "home-relay-key",
    },
    confirmedAt: "2026-09-11T00:00:00Z",
  }
  const receiptBytes = `${JSON.stringify(receipt, null, 2)}\n`
  const activeReceiptPath = harness.receiptPath
  await chmod(dirname(activeReceiptPath), 0o700)
  await put(activeReceiptPath, receiptBytes, 0o640)
  const before = await persistentSnapshot(harness.persistent)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readFile(activeReceiptPath, "utf8"), receiptBytes)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  const serviceLog = await readFile(join(harness.state, "systemctl.log"), "utf8")
  assert.match(serviceLog, /start chariox-disposable-worker-bootstrap.service/)
  assert.doesNotMatch(serviceLog, /(?:start|stop) chariox-managed-bootstrap.service/)
  const rollback = harness.run({}, [harness.current.rootfs, harness.target.digest, harness.current.digest, harness.trustedKey])
  assert.equal(rollback.status, 0, rollback.stderr)
  assert.equal(await readFile(activeReceiptPath, "utf8"), receiptBytes)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
})

test("legacy worker upgrade is rejected before service mutation", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "disposable_worker" })
  const receipt = await readFile(harness.receiptPath, "utf8")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /legacy disposable worker bootstrap is unsupported/)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receipt)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade preserves an exact disposable worker receipt and persistent state", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const before = await persistentSnapshot(harness.persistent)
  const receiptBefore = await readFile(harness.receiptPath, "utf8")
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receiptBefore)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  assert.deepEqual(
    JSON.parse(await readFile(join(dirname(harness.receiptPath), "release-override.json"), "utf8")),
    {
      schemaVersion: 1,
      kind: "disposable_worker_release",
      bindingDigest: harness.bindingDigest,
      runtimeReleaseDigest: harness.target.digest,
    },
  )
})

test("allocation-worker upgrade and recovery preserve install-time App storage enrollment", async context => {
  const harness = await makeHarness(context, {receiptKind: "allocation_worker", path1Release: true})
  const config = {schema: "chariox.app-storage-enrollment.v1", owners: [{
    uid: harness.charioxIdentity.uid, gid: harness.charioxIdentity.gid,
    cgroup_root: "/sys/fs/cgroup/system.slice/chariox-path1-managed-bootstrap.service/apps",
    kernel_database_paths: ["/home/chariox/.chariox/state/kernel.db"],
  }]}
  const enrollment = join(harness.installRoot, "etc/chariox/app-storage.json")
  await put(enrollment, `${JSON.stringify(config)}\n`, 0o644)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const failed = harness.run({CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1", CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey})
  assert.equal(failed.status, 1)
  assert.match(failed.stderr, /restored previous managed kernel release/)
  assert.deepEqual(JSON.parse(await readFile(enrollment, "utf8")), config)
  const succeeded = harness.run({CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1", CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey})
  assert.equal(succeeded.status, 0, succeeded.stderr)
  assert.deepEqual(JSON.parse(await readFile(enrollment, "utf8")), config)
})

test("ordinary managed upgrade rejects a worker release override before service mutation", async (context) => {
  const harness = await makeHarness(context)
  await put(
    join(dirname(harness.receiptPath), "release-override.json"),
    `${JSON.stringify({
      schemaVersion: 1,
      kind: "disposable_worker_release",
      bindingDigest: `sha256:${"a".repeat(64)}`,
      runtimeReleaseDigest: harness.current.digest,
    })}\n`,
    0o640,
  )
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /ordinary managed bootstrap receipt cannot use/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("disposable worker upgrade rejects altered Cloud authority before service mutation", async (context) => {
  for (const mutate of [
    (receipt) => { receipt.homeCaller.kernelId = "other-home" },
    (receipt) => { receipt.status = "exchanged" },
    (receipt) => { receipt.homeCaller.accountId = "other-account" },
    (receipt) => { receipt.unexpected = true },
  ]) {
    const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
    const receipt = JSON.parse(await readFile(harness.receiptPath, "utf8"))
    await put(join(dirname(harness.receiptPath), "release-override.json"), JSON.stringify({
      schemaVersion: 1, kind: "disposable_worker_release", bindingDigest: harness.bindingDigest,
      runtimeReleaseDigest: harness.current.digest,
    }), 0o640)
    mutate(receipt)
    await put(harness.receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, 0o640)
    const result = harness.run()
    assert.equal(result.status, 1)
    assert.match(result.stderr, /allocation worker bootstrap receipt is invalid|disposable worker release override is invalid/)
    assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
  }
})

test("disposable worker rollback restores its prior signed-release override", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const overridePath = join(dirname(harness.receiptPath), "release-override.json")
  await put(overridePath, `${JSON.stringify({
    schemaVersion: 1,
    kind: "disposable_worker_release",
    bindingDigest: harness.bindingDigest,
    runtimeReleaseDigest: harness.current.digest,
  }, null, 2)}\n`, 0o640)
  const before = await readFile(overridePath, "utf8")
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(await readFile(overridePath, "utf8"), before)
})

test("disposable worker rollback removes a newly staged release override", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const receiptBefore = await readFile(harness.receiptPath, "utf8")
  const overridePath = join(dirname(harness.receiptPath), "release-override.json")
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(await lstat(overridePath).then(() => true, () => false), false)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receiptBefore)
})

test("disposable worker upgrade recovers an interruption without rewriting Cloud authority", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const receiptBefore = await readFile(harness.receiptPath, "utf8")
  await put(join(harness.state, "crash-before-symlink"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receiptBefore)
  assert.equal(
    JSON.parse(await readFile(join(dirname(harness.receiptPath), "release-override.json"), "utf8"))
      .runtimeReleaseDigest,
    harness.target.digest,
  )
})

test("managed kernel health rejects an unrelated listener without a fresh matching kernel presence", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "skip-presence-once"), "skip\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel health uses the installed home presence directory for upgrade and rollback", async (context) => {
  const harness = await makeHarness(context)
  const presenceRoot = join(harness.installRoot, "home/chariox/.chariox/kernels/active")
  await put(join(harness.state, "skip-presence-once"), "skip\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.ok(result.stderr.includes(`presence directory ${presenceRoot}`), result.stderr)
  assert.doesNotMatch(result.stderr, /home\/\.chariox\/kernels\/active/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(
    await lstat(join(presenceRoot, "kernel-1.json")).then(() => true, () => false),
    true,
  )
})

test("managed kernel health rejects stale, wrong-machine, or oversized presence", async (context) => {
  for (const marker of ["stale-presence-once", "wrong-machine-once", "oversized-presence-once"]) {
    const harness = await makeHarness(context)
    await put(join(harness.state, marker), "reject\n")
    const result = harness.run()
    assert.equal(result.status, 1)
    assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  }
})

test("managed kernel health is bound to the expected release digest", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "wrong-release-once"), "reject\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.doesNotMatch(result.stderr, /final receipt validation failed/)
})

test("managed kernel readiness preserves the registered environment identity", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "wrong-environment-once"), "reject\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /final receipt validation failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.deepEqual(JSON.parse(await readFile(harness.receiptPath, "utf8")), harness.receipt)
})

test("managed kernel upgrade rejects writable release authority and receipt state before stopping service", async (context) => {
  const writableReleaseRoot = await makeHarness(context)
  await chmod(join(writableReleaseRoot.installRoot, "usr/lib/chariox/releases"), 0o777)
  let result = writableReleaseRoot.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed kernel upgrade authority permissions are unsafe/)
  assert.equal(await lstat(join(writableReleaseRoot.state, "systemctl.log")).then(() => true, () => false), false)

  const writableCurrentRelease = await makeHarness(context)
  await chmod(
    join(
      writableCurrentRelease.installRoot,
      "usr/lib/chariox/releases",
      writableCurrentRelease.current.digest.slice("sha256:".length),
    ),
    0o777,
  )
  result = writableCurrentRelease.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed kernel upgrade authority permissions are unsafe/)
  assert.equal(await lstat(join(writableCurrentRelease.state, "systemctl.log")).then(() => true, () => false), false)

  const writableReceipt = await makeHarness(context)
  await chmod(writableReceipt.receiptPath, 0o660)
  result = writableReceipt.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt permissions are unsafe/)
  assert.equal(await lstat(join(writableReceipt.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rolls the binary and receipt back together when health fails", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  // The target supervisor rebinds the grant before health; the previous one must read its own.
  const grantBinding = join(dirname(harness.receiptPath), "bootstrap-grant-binding.json")
  await put(grantBinding, '{"schemaVersion":1}\n', 0o600)
  await put(join(harness.state, "rebind-grant-once"), "rebind\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
  assert.equal(await readFile(grantBinding, "utf8"), '{"schemaVersion":1}\n')
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.deepEqual(calls.slice(-7), [
    `stop ${serviceName}`,
    "daemon-reload",
    "enable chariox-app-storage.service",
    "restart chariox-app-storage.service",
    "is-active --quiet chariox-app-storage.service",
    `start ${serviceName}`,
    `is-active --quiet ${serviceName}`,
  ])
})

test("managed kernel rollback never reports success when restored health is unverified", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-always"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /rollback remains pending/)
  assert.doesNotMatch(result.stderr, /restored previous managed kernel release/)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
      .then((metadata) => metadata.isDirectory(), () => false),
    true,
  )
})

test("managed kernel upgrade immediately rolls back a post-health receipt validation failure", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-final-receipt-validation"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /final receipt validation failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
      .then(() => true, () => false),
    false,
  )
})

test("managed kernel upgrade recovers an interruption between activation stages before retrying", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "crash-before-symlink"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const transaction = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade")
  assert.equal((await stat(transaction)).isDirectory(), true)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.target.digest)

  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.target.digest)
  assert.equal(await lstat(transaction).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects digest mismatch, a stale precondition, and a partial or unsigned release", async (context) => {
  const badDigest = await makeHarness(context)
  let result = badDigest.run({}, [
    badDigest.target.rootfs,
    badDigest.current.digest,
    `sha256:${"0".repeat(64)}`,
    badDigest.trustedKey,
  ])
  assert.equal(result.status, 1)
  assert.match(result.stderr, /release manifest digest does not match/)

  const staleRequest = await makeHarness(context)
  result = staleRequest.run({}, [
    staleRequest.target.rootfs,
    `sha256:${"f".repeat(64)}`,
    staleRequest.target.digest,
    staleRequest.trustedKey,
  ])
  assert.equal(result.status, 1)
  assert.match(result.stderr, /installed release does not match the expected current release/)

  const alreadyCurrent = await makeHarness(context)
  result = alreadyCurrent.run({}, [
    alreadyCurrent.current.rootfs,
    alreadyCurrent.current.digest,
    alreadyCurrent.current.digest,
    alreadyCurrent.trustedKey,
  ])
  assert.equal(result.status, 1)
  assert.match(result.stderr, /target release is not newer than current/)

  const partial = await makeHarness(context)
  await rm(join(partial.target.rootfs, "usr/local/bin/chariox-managed-bootstrap"))
  result = partial.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /cannot be read|invalid file|corrupted/)

  const unsigned = await makeHarness(context)
  await put(join(unsigned.target.rootfs, "usr/lib/chariox/release-manifest.sig"), Buffer.alloc(64).toString("base64"))
  result = unsigned.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /release signature is invalid/)

  const tampered = await makeHarness(context)
  await put(join(tampered.target.rootfs, "usr/local/bin/chariox-kernel"), "#!/bin/sh\nexit 0\n", 0o755)
  result = tampered.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /release artifact chariox-kernel is corrupted/)
})

test("managed kernel upgrade rejects linked authority paths before service mutation", async (context) => {
  const linkedReceipt = await makeHarness(context)
  const receiptContents = await readFile(linkedReceipt.receiptPath)
  const externalReceipt = join(linkedReceipt.root, "external-receipt.json")
  await put(externalReceipt, receiptContents, 0o600)
  await rm(linkedReceipt.receiptPath)
  await symlink(externalReceipt, linkedReceipt.receiptPath)
  let result = linkedReceipt.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /invalid file/)
  assert.equal(await lstat(join(linkedReceipt.state, "systemctl.log")).then(() => true, () => false), false)

  const linkedTransaction = await makeHarness(context)
  const externalTransaction = join(linkedTransaction.root, "external-transaction")
  await mkdir(externalTransaction)
  await symlink(
    externalTransaction,
    join(linkedTransaction.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"),
  )
  result = linkedTransaction.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /transaction path is obstructed/)
  assert.equal(await lstat(join(linkedTransaction.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects writable or linked authority ancestors before service mutation", async (context) => {
  const writableReceiptAncestor = await makeHarness(context)
  await chmod(dirname(writableReceiptAncestor.receiptPath), 0o777)
  let result = writableReceiptAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt ancestor permissions are unsafe/)
  assert.equal(await lstat(join(writableReceiptAncestor.state, "systemctl.log")).then(() => true, () => false), false)

  const linkedReceiptAncestor = await makeHarness(context)
  const managedDirectory = dirname(linkedReceiptAncestor.receiptPath)
  const externalManagedDirectory = join(linkedReceiptAncestor.root, "external-managed")
  await rename(managedDirectory, externalManagedDirectory)
  await symlink(externalManagedDirectory, managedDirectory)
  result = linkedReceiptAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt ancestor is unsafe/)
  assert.equal(await lstat(join(linkedReceiptAncestor.state, "systemctl.log")).then(() => true, () => false), false)

  const writableKeyAncestor = await makeHarness(context)
  await chmod(dirname(writableKeyAncestor.trustedKey), 0o777)
  result = writableKeyAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /trusted release public key ancestor permissions are unsafe/)
  assert.equal(await lstat(join(writableKeyAncestor.state, "systemctl.log")).then(() => true, () => false), false)

  const foreignReceiptAncestor = await makeHarness(context)
  await put(join(foreignReceiptAncestor.state, "foreign-receipt-ancestor"), "simulate\n")
  result = foreignReceiptAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt ancestor owner is unsafe/)
  assert.equal(await lstat(join(foreignReceiptAncestor.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects a non-root-owned trusted release key", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "non-root-key-owner"), "simulate\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /trusted release public key owner is unsafe/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade requires the exact confirmed registered-kernel receipt", async (context) => {
  const malformed = await makeHarness(context)
  const malformedReceipt = JSON.parse(await readFile(malformed.receiptPath, "utf8"))
  malformedReceipt.futureAuthority = { allowProvisioning: true }
  await put(malformed.receiptPath, `${JSON.stringify(malformedReceipt)}\n`, 0o600)
  let result = malformed.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt contains unsupported fields/)

  const unconfirmed = await makeHarness(context)
  const unconfirmedReceipt = JSON.parse(await readFile(unconfirmed.receiptPath, "utf8"))
  unconfirmedReceipt.status = "exchanged"
  unconfirmedReceipt.confirmedAt = null
  await put(unconfirmed.receiptPath, `${JSON.stringify(unconfirmedReceipt)}\n`, 0o600)
  result = unconfirmed.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not a confirmed registered-kernel receipt/)
})

test("managed kernel upgrade accepts the schema 3 receipt a Path-1 kernel writes", async (context) => {
  const harness = await makeHarness(context)
  const receipt = { ...harness.receipt, schemaVersion: 3, generation: 2, managedRepositoryRoot: "/home/chariox" }
  await put(harness.receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, 0o640)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(
    JSON.parse(await readFile(harness.receiptPath, "utf8")),
    { ...receipt, runtimeReleaseDigest: harness.target.digest },
  )

  const schema2 = await makeHarness(context)
  const schema2Receipt = { ...schema2.receipt, schemaVersion: 2, managedRepositoryRoot: "/home/chariox/work" }
  await put(schema2.receiptPath, `${JSON.stringify(schema2Receipt)}\n`, 0o640)
  const schema2Result = schema2.run()
  assert.equal(schema2Result.status, 0, schema2Result.stderr)

  for (const [label, change] of [
    ["schema 3 without a repository root", { managedRepositoryRoot: undefined }],
    ["schema 1 with a repository root", { schemaVersion: 1 }],
    ["generation 0", { generation: 0 }],
    ["release-bound freshness evidence", { freshnessEvidence: {} }],
  ]) {
    const rejected = await makeHarness(context)
    await put(rejected.receiptPath, `${JSON.stringify({ ...receipt, ...change })}\n`, 0o640)
    assert.equal(rejected.run().status, 1, label)
  }
})

// This signed installer fixture is not proof of real-binary state migration.
for (const [currentProtocol, targetProtocol] of [[410, 411], [411, 415], [415, 416]]) {
test(`managed kernel upgrade accepts the signed repository ${currentProtocol} to ${targetProtocol} fixture transition and rollback`, async (context) => {
  const repositoryPolicy = JSON.parse(await readFile(join(repositoryRoot, "apps/kernel/managed-upgrade-protocol-transitions.json"), "utf8"))
  const harness = await makeHarness(context, {
    currentProtocol,
    currentTransitionPolicy: { ...repositoryPolicy, protocol: currentProtocol,
      upgradeFrom: repositoryPolicy.upgradeFrom.filter(version => version <= currentProtocol),
      rollbackTo: repositoryPolicy.rollbackTo.filter(version => version <= currentProtocol) },
    targetProtocol,
    targetTransitionPolicy: { ...repositoryPolicy, protocol: targetProtocol,
      upgradeFrom: repositoryPolicy.upgradeFrom.filter(version => version <= targetProtocol),
      rollbackTo: repositoryPolicy.rollbackTo.filter(version => version <= targetProtocol) },
  })
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  const rollback = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(rollback.status, 0, rollback.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
})

}

test("managed kernel upgrade rejects signed ambiguous protocol 351 before stopping services", async (context) => {
  const repositoryPolicy = JSON.parse(await readFile(join(repositoryRoot, "apps/kernel/managed-upgrade-protocol-transitions.json"), "utf8"))
  const harness = await makeHarness(context, {
    currentProtocol: 351,
    targetProtocol: repositoryPolicy.protocol,
    targetTransitionPolicy: repositoryPolicy,
  })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not reciprocally authorized/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel upgrade rejects a forward transition authorized on only one leg", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 322,
    targetProtocol: 323,
    targetTransitionPolicy: {
      schemaVersion: 1,
      protocol: 323,
      upgradeFrom: [322, 323],
      rollbackTo: [323],
    },
  })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not reciprocally authorized/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects a reverse transition authorized on only one leg", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 323,
    targetProtocol: 322,
    currentTransitionPolicy: {
      schemaVersion: 1,
      protocol: 323,
      upgradeFrom: [323],
      rollbackTo: [322, 323],
    },
  })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not reciprocally authorized/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects an unsupported local daemon protocol transition", async (context) => {
  const harness = await makeHarness(context, { targetProtocol: 322 })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /protocol transition policy is missing or invalid/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("a signed transition policy permits post-success rollback to its declared protocol", async (context) => {
  const policy = {
    schemaVersion: 1,
    protocol: 323,
    upgradeFrom: [322, 323],
    rollbackTo: [322, 323],
  }
  const harness = await makeHarness(context, {
    currentProtocol: 322,
    targetProtocol: 323,
    targetTransitionPolicy: policy,
  })
  const upgraded = harness.run()
  assert.equal(upgraded.status, 0, upgraded.stderr)
  const rolledBack = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(rolledBack.status, 0, rolledBack.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
})

test("Apps rollback requires an explicit override and warns before pre-Apps activation", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 376, targetProtocol: 410,
    targetTransitionPolicy: {schemaVersion: 1, protocol: 410, upgradeFrom: [376, 410], rollbackTo: [376, 410]},
  })
  const upgraded = harness.run()
  assert.equal(upgraded.status, 0, upgraded.stderr)
  const database = join(harness.installRoot, "home/chariox/.chariox/state/kernel.db")
  await mkdir(dirname(database), {recursive: true})
  const createState = spawnSync("python3", ["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('CREATE TABLE app_state_migrations (installation_id TEXT)'); c.commit(); c.close()", database], {encoding: "utf8"})
  assert.equal(createState.status, 0, createState.stderr)
  const before = await readFile(database)
  const args = [harness.current.rootfs, harness.target.digest, harness.current.digest, harness.trustedKey]
  const refused = harness.run({}, args)
  assert.equal(refused.status, 1)
  assert.match(refused.stderr, /Apps boundary with App state is blocked/)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`)
  const allowed = harness.run({}, ["--allow-apps-rollback", ...args])
  assert.equal(allowed.status, 0, allowed.stderr)
  assert.match(allowed.stderr, /WARNING:.*App state survival/)
  assert.deepEqual(await readFile(database), before)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`)
})

test("a failed overridden Apps downgrade restores the previous Apps release", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 376, targetProtocol: 410,
    targetTransitionPolicy: {schemaVersion: 1, protocol: 410, upgradeFrom: [376, 410], rollbackTo: [376, 410]},
  })
  assert.equal(harness.run().status, 0)
  const database = join(harness.installRoot, "home/chariox/.chariox/state/kernel.db")
  await mkdir(dirname(database), {recursive: true})
  const state = spawnSync("python3", ["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('CREATE TABLE app_state_migrations(installation_id TEXT)'); c.commit(); c.close()", database], {encoding: "utf8"})
  assert.equal(state.status, 0, state.stderr)
  const before = await readFile(database)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const downgrade = harness.run({}, ["--allow-apps-rollback", harness.current.rootfs,
    harness.target.digest, harness.current.digest, harness.trustedKey])
  assert.equal(downgrade.status, 1)
  assert.match(downgrade.stderr, /WARNING:.*App state survival/)
  assert.deepEqual(await readFile(database), before)
  assert.match(downgrade.stderr, /restored previous managed kernel release/)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`)
  assert.equal(await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade")).then(() => true, () => false), false)
})

test("failed upgrade with no App state automatically rolls back across the boundary", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 376, targetProtocol: 410,
    targetTransitionPolicy: {schemaVersion: 1, protocol: 410, upgradeFrom: [376, 410], rollbackTo: [376, 410]},
  })
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /restored previous managed kernel release/)
  assert.doesNotMatch(result.stderr, /Apps boundary|Apps rollback override/)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`)
})

const legacyUpdaterCommit = "8fa9246ea60b52a988da17a28652e61475ff52cd"

async function legacyUpdater() {
  const root = join(repositoryRoot, "scripts/fixtures/managed-kernel-upgrade-8fa9246")
  const provenance = JSON.parse(await readFile(join(root, "provenance.json"), "utf8"))
  assert.equal(provenance.sourceCommit, legacyUpdaterCommit)
  for (const [name, source] of Object.entries(provenance.files)) {
    assert.equal(source.sourcePath, `deploy/managed-kernel/${name}`)
    assert.equal(createHash("sha256").update(await readFile(join(root, name))).digest("hex"), source.sha256,
      `historical updater fixture must retain exact ${legacyUpdaterCommit} bytes: ${name}`)
  }
  return join(root, "upgrade-image.sh")
}

test("the exact legacy installed updater rejects the current signed target before commit or interruption", async (context) => {
  const updaterPath = await legacyUpdater()
  for (const mode of ["commit", "interruption"]) {
    // The legacy updater's installed release predates Apps; only the target carries the App set.
    const harness = await makeHarness(context, {
      currentManifestSchema: 2, targetManifestSchema: 3, currentAppArtifacts: false, updaterPath,
    })
    const attemptPath = join(harness.installRoot, "home/chariox/.chariox/release-update-attempt.json")
    const legacyAttempt = { updateId: cloudUpdateId, targetRuntimeReleaseDigest: harness.target.digest }
    await put(attemptPath, `${JSON.stringify(legacyAttempt)}\n`, 0o600)
    if (mode === "interruption") await put(join(harness.state, "crash-after-supervisor-start"), "crash\n")
    const result = harness.run()
    assert.equal(result.status, 1, `${mode}: old installed tooling must reject before starting target B`)
    assert.match(result.stderr, /release manifest contains unsupported fields|release manifest schema is unsupported/)
    assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
    assert.deepEqual(JSON.parse(await readFile(attemptPath, "utf8")), legacyAttempt)
    assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
    assert.equal(await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade-result"))
      .then(() => true, () => false), false)
  }
})

const cloudUpdateId = "managed_release_update_0123abcd-0000-4000-8000-0123456789ab"

function expectedUpdateResult(harness, phase) {
  return [
    "1", cloudUpdateId, harness.current.digest, harness.target.digest,
    harness.receipt.environmentId, harness.receipt.machineId, harness.receipt.kernelId, phase,
  ].join("\n") + "\n"
}

test("Cloud updates reject legacy targets before release or service mutation", async (context) => {
  const harness = await makeHarness(context, { targetManifestSchema: 2 })
  const result = harness.run({ CHARIOX_MANAGED_RELEASE_UPDATE_ID: cloudUpdateId })
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /target requires signed update evidence capability 1/)
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
  assert.equal(await lstat(join(harness.installRoot, "usr/lib/chariox/releases", harness.target.digest.slice(7)))
    .then(() => true, () => false), false)
  assert.equal(await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
    .then(() => true, () => false), false)
})

test("schema 3 tooling retains verified legacy schema 2 rollback", async (context) => {
  const harness = await makeHarness(context, { currentManifestSchema: 2, targetManifestSchema: 3 })
  const upgraded = harness.run()
  assert.equal(upgraded.status, 0, upgraded.stderr)
  const rolledBack = harness.run({}, [
    harness.current.rootfs, harness.target.digest, harness.current.digest, harness.trustedKey,
  ])
  assert.equal(rolledBack.status, 0, rolledBack.stderr)
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
})

test("Cloud update terminal evidence survives committed journal cleanup", async (context) => {
  const harness = await makeHarness(context)
  const result = harness.run({ CHARIOX_MANAGED_RELEASE_UPDATE_ID: cloudUpdateId })
  assert.equal(result.status, 0, result.stderr)
  const evidence = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade-result")
  assert.equal(await readFile(evidence, "utf8"), expectedUpdateResult(harness, "committed"))
  assert.equal((await stat(evidence)).mode & 0o777, 0o644)
  assert.equal(await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
    .then(() => true, () => false), false)
  const nextUpdateId = "managed_release_update_0123abcd-0000-4000-8000-0123456789ac"
  const reverse = harness.run({ CHARIOX_MANAGED_RELEASE_UPDATE_ID: nextUpdateId }, [
    harness.current.rootfs, harness.target.digest, harness.current.digest, harness.trustedKey,
  ])
  assert.equal(reverse.status, 0, reverse.stderr)
  assert.equal(await readFile(evidence, "utf8"), [
    "1", nextUpdateId, harness.target.digest, harness.current.digest,
    harness.receipt.environmentId, harness.receipt.machineId, harness.receipt.kernelId, "committed",
  ].join("\n") + "\n")
})

test("Cloud update recovers committed evidence after result or cleanup interruption", async (context) => {
  for (const marker of ["crash-after-phase-committed", "crash-after-committed-tombstone"]) {
    const harness = await makeHarness(context)
    const evidence = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade-result")
    await put(join(harness.state, marker), "crash\n")
    const interrupted = harness.run({ CHARIOX_MANAGED_RELEASE_UPDATE_ID: cloudUpdateId })
    assert.equal(interrupted.signal, "SIGKILL", marker)
    assert.equal(await lstat(evidence).then(() => true, () => false), marker.includes("tombstone"))
    const recovered = harness.run({}, [harness.target.rootfs, `sha256:${"f".repeat(64)}`, harness.current.digest, harness.trustedKey])
    assert.equal(recovered.status, 1, marker)
    assert.match(recovered.stderr, /installed release does not match/)
    assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.target.digest)
    assert.equal(await readFile(evidence, "utf8"), expectedUpdateResult(harness, "committed"))
  }
})

test("Cloud update evidence stays pending between supervisor start and rollback", async (context) => {
  const harness = await makeHarness(context)
  const evidence = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade-result")
  await put(join(harness.state, "crash-after-supervisor-start"), "crash\n")
  const interrupted = harness.run({ CHARIOX_MANAGED_RELEASE_UPDATE_ID: cloudUpdateId })
  assert.equal(interrupted.signal, "SIGKILL")
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.target.digest)
  assert.equal(await readFile(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade/phase"), "utf8"), "activated\n")
  assert.equal(await lstat(evidence).then(() => true, () => false), false)
  // Recover the old transaction, then reject this request before starting a new one.
  const recovered = harness.run({}, [harness.target.rootfs, `sha256:${"f".repeat(64)}`, harness.target.digest, harness.trustedKey])
  assert.equal(recovered.status, 1)
  assert.match(recovered.stderr, /installed release does not match/)
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
  assert.equal(await readFile(evidence, "utf8"), expectedUpdateResult(harness, "rolled_back"))
})

test("managed kernel upgrade recovers interruption after every persisted nonterminal phase", async (context) => {
  for (const phase of ["prepared", "stopped", "activated"]) {
    const harness = await makeHarness(context)
    await put(join(harness.state, `crash-after-phase-${phase}`), "crash\n")
    const interrupted = harness.run()
    assert.equal(interrupted.signal, "SIGKILL", phase)
    const retried = harness.run()
    assert.equal(retried.status, 0, `${phase}: ${retried.stderr}`)
    assert.equal(
      await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
      `releases/${harness.target.digest.slice("sha256:".length)}`,
      phase,
    )
  }
})

test("managed kernel upgrade recovers interruption after the persisted rolled-back phase", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  await put(join(harness.state, "crash-after-phase-rolled_back"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
})

test("cross-protocol recovery authorizes automatic rollback and re-upgrade before shutdown", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 322,
    targetProtocol: 323,
    targetTransitionPolicy: {
      schemaVersion: 1,
      protocol: 323,
      upgradeFrom: [322, 323],
      rollbackTo: [322, 323],
    },
  })
  await put(join(harness.state, "crash-after-phase-activated"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.equal(calls.filter((call) => call === `stop ${serviceName}`).length, 3)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel upgrade recovers interruption after the persisted committed phase", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "crash-after-phase-committed"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const recovered = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(recovered.status, 0, recovered.stderr)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
      .then(() => true, () => false),
    false,
  )
})

test("managed kernel upgrade discards a committed tombstone after cleanup interruption", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "crash-after-committed-tombstone"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const tombstone = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade.terminal")
  assert.equal((await stat(tombstone)).isDirectory(), true)
  const recovered = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(recovered.status, 0, recovered.stderr)
  assert.equal(await lstat(tombstone).then(() => true, () => false), false)
})

test("managed kernel upgrade discards a rolled-back tombstone after cleanup interruption", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  await put(join(harness.state, "crash-after-rolled_back-tombstone"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const tombstone = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade.terminal")
  assert.equal((await stat(tombstone)).isDirectory(), true)
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(await lstat(tombstone).then(() => true, () => false), false)
})

test("managed kernel upgrade remains a dedicated offline release operation", async () => {
  const contents = await readFile(upgrade, "utf8")
  const stateContents = await readFile(upgradeState, "utf8")
  const serviceContents = await readFile(managedService, "utf8")
  assert.match(contents, /verify-image-release\.mjs/)
  assert.match(contents, /--print-local-daemon-protocol-version/)
  assert.match(contents, /systemctl stop "\$service_name"/)
  assert.match(contents, /systemctl start "\$service_name"/)
  assert.doesNotMatch(contents, /install-image\.sh/)
  assert.doesNotMatch(contents, /\b(?:groupadd|useradd|usermod|loginctl)\b/)
  assert.match(contents, /systemctl enable chariox-app-storage.service/)
  assert.match(contents, /systemctl restart chariox-app-storage.service/)
  assert.doesNotMatch(contents, /\b(?:curl|wget|ssh|scp)\b/)
  assert.doesNotMatch(contents, /installation[_-]origin|CHARIOX_INSTALLATION/)
  assert.doesNotMatch(contents, /\.arroba/)
  assert.match(contents, /CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS:-120000/)
  assert.match(contents, /managed_provider_topology=\$\{CHARIOX_MANAGED_PROVIDER_TOPOLOGY-\}/)
  assert.match(contents, /service_name=chariox-path1-managed-bootstrap\.service/)
  assert.match(contents, /select_supervisor_service\(\)/)
  assert.match(contents, /verify_selected_release\(\)/)
  assert.match(contents, /path1_unit=chariox-path1-managed-bootstrap\.service/)
  assert.match(serviceContents, /ExecStartPre=-\+\/usr\/bin\/systemctl restart chariox-slice-broker\.service/)

  const publishRelease = contents.indexOf('mv "$pending_release" "$published_release"')
  const publishTransaction = contents.indexOf('publish-transaction \\', publishRelease)
  const stopService = contents.indexOf('systemctl stop "$service_name"', publishTransaction)
  assert.ok(publishRelease >= 0 && publishTransaction > publishRelease && stopService > publishTransaction)
  const verifyCandidate = contents.indexOf('verify_selected_release "$image_root"', 0)
  const planMigration = contents.indexOf('\nplan_home_migration\n', verifyCandidate)
  const applyMigration = contents.indexOf('resume_home_migration', stopService)
  assert.ok(verifyCandidate >= 0 && planMigration > verifyCandidate)
  assert.ok(planMigration < publishTransaction && applyMigration > stopService)
  assert.match(contents.slice(0, publishRelease), /sync-tree[^\n]*\$pending_release/)
  assert.match(contents.slice(publishRelease, publishTransaction), /sync-directory[^\n]*\$releases_root/)
  assert.match(contents.slice(publishRelease, publishTransaction), /sync-tree[^\n]*\$pending_transaction/)
  assert.match(contents.slice(publishTransaction, stopService), /publish-transaction/)
  assert.match(stateContents, /await rename\(source, destination\)\n  await fsyncDirectory\(dirname\(destination\)\)/)
})


test("App enrollment propagates directory creation failure inside a conditional caller", async context => {
  const root = await mkdtemp(join(tmpdir(), "chariox-app-enroll-failure-"))
  context.after(() => rm(root, {recursive: true, force: true}))
  const bin = join(root, "bin")
  await put(join(bin, "id"), "#!/bin/sh\necho 1000\n", 0o755)
  await put(join(bin, "install"), "#!/bin/sh\nexit 42\n", 0o755)
  const helper = join(repositoryRoot, "deploy/managed-kernel/managed-app-storage.sh")
  const result = spawnSync("sh", ["-c", '. "$1"; enroll_managed_app_storage "$2" shared_host || exit 23', "test", helper, join(root, "host")], {
    env: {...process.env, PATH: `${bin}:${process.env.PATH}`}, encoding: "utf8",
  })
  assert.equal(result.status, 23)
  assert.equal(await lstat(join(root, "host/etc/chariox/app-storage.json")).then(() => true, () => false), false)
})

}
