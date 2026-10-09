// MP-04/MP-07/MP-10/MP-11: disposable signed-release fixture; private test keys stay in memory.
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { spawnSync } from "node:child_process"
import {
  chmod,
  chown,
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  readdir,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import {
  dirname,
  join,
  relative,
  sep,
} from "node:path"
import { fileURLToPath } from "node:url"
import { allocationWorkerBindingDigest } from "../../deploy/managed-kernel/allocation-worker-receipt.mjs"

const repositoryRoot = fileURLToPath(new URL("../..", import.meta.url))
const upgrade = join(repositoryRoot, "deploy/managed-kernel/upgrade-image.sh")
const serviceName = "chariox-managed-bootstrap.service"

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: "der", type: "spki" })
  return der.subarray(der.length - 32)
}

async function sha256File(path) {
  return `sha256:${createHash("sha256").update(await readFile(path)).digest("hex")}`
}

async function updateTreeHash(root, current, hash) {
  for (const entry of await readdir(current, { withFileTypes: true }).then((entries) =>
    entries.sort((left, right) => left.name.localeCompare(right.name)),
  )) {
    const path = join(current, entry.name)
    const pathFromRoot = relative(root, path).split(sep).join("/")
    const metadata = await lstat(path)
    const mode = metadata.mode & 0o7777
    if (entry.isDirectory()) {
      hash.update(`directory:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:`)
      await updateTreeHash(root, path, hash)
    } else {
      hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
      hash.update(await readFile(path))
    }
  }
}

async function sha256Tree(root) {
  const hash = createHash("sha256")
  const metadata = await lstat(root)
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}

async function put(path, contents, mode = 0o644) {
  await mkdir(dirname(path), { recursive: true, mode: 0o755 })
  await writeFile(path, contents, { mode })
  await chmod(path, mode)
}

async function createRootPrivateDirectory(path) {
  await mkdir(path, { recursive: true, mode: 0o700 })
  if (process.getuid?.() === 0) await chown(path, 0, 0)
  await chmod(path, 0o700)
}

function namespaceMapsId(contents, id) {
  return contents.trim().split("\n").some((line) => {
    const [inside, , length] = line.trim().split(/\s+/).map(Number)
    return Number.isSafeInteger(inside) && Number.isSafeInteger(length)
      && id >= inside && id < inside + length
  })
}

async function effectiveCharioxIdentity() {
  const uid = Number(spawnSync("id", ["-u", "chariox"], { encoding: "utf8" }).stdout.trim() || "1000")
  const gid = Number(spawnSync("id", ["-g", "chariox"], { encoding: "utf8" }).stdout.trim() || "1000")
  if (process.platform !== "linux") return { uid, gid }
  const [uidMap, gidMap] = await Promise.all([
    readFile("/proc/self/uid_map", "utf8").catch(() => ""),
    readFile("/proc/self/gid_map", "utf8").catch(() => ""),
  ])
  if (namespaceMapsId(uidMap, uid) && namespaceMapsId(gidMap, gid)) return { uid, gid }
  return { uid: 0, gid: 0 }
}

async function makeRelease(root, label, protocol, privateKey, publicKey, transitionPolicy = null, includeWorkerService = false, builderKeys = null, manifestSchema = 3, includeAppArtifacts = true) {
  const rootfs = join(root, `image-${label}`)
  const kernel = join(rootfs, "usr/local/bin/chariox-kernel")
  const supervisor = join(rootfs, "usr/local/bin/chariox-managed-bootstrap")
  // Releases built before Apps carry no App binaries or App storage unit.
  const appPackage = join(rootfs, "usr/local/bin/chariox-app-package")
  const appStorage = join(rootfs, "usr/libexec/chariox-app-storage")
  const appStorageService = join(rootfs, "etc/systemd/system/chariox-app-storage.service")
  const managedService = join(rootfs, `etc/systemd/system/${serviceName}`)
  const rootlessService = join(rootfs, "etc/systemd/system/chariox-rootless-docker.service")
  const brokerService = join(rootfs, "etc/systemd/system/chariox-slice-broker.service")
  const context = join(rootfs, "usr/lib/chariox/slice-build-context")
  const attestation = join(rootfs, "usr/lib/chariox/build-attestation.json")
  const attestationSignature = join(rootfs, "usr/lib/chariox/build-attestation.sig")
  const builderKey = join(rootfs, "usr/lib/chariox/builder-public-key")
  const dataVolumeAdmissionService = join(repositoryRoot, "apps/kernel/slice-linux-docker/chariox-data-volume-admission.service")
  const rootlessDataVolumeDropIn = join(repositoryRoot, "apps/kernel/slice-linux-docker/chariox-rootless-docker.path1-data-volume.conf")
  const allocatorDataVolumeDropIn = join(repositoryRoot, "apps/kernel/slice-linux-docker/chariox-slice-disk-quota-allocator.path1-data-volume.conf")
  await put(kernel, `#!/bin/sh\nif [ "\$1" = "--print-local-daemon-protocol-version" ]; then echo ${protocol}; exit 0; fi\nexit 1\n`, 0o755)
  await put(supervisor, `#!/bin/sh\necho supervisor-${label}\n`, 0o755)
  if (includeAppArtifacts) {
    await put(appPackage, `#!/bin/sh\necho app-package-${label}\n`, 0o755)
    await put(appStorage, `#!/bin/sh\necho app-storage-${label}\n`, 0o755)
    await put(appStorageService, await readFile(join(repositoryRoot, "deploy/managed-kernel/chariox-app-storage.service")))
  }
  await put(managedService, `[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n# ${label}\n`)
  await put(rootlessService, `[Service]\n# rootless ${label}\n`)
  await put(brokerService, `[Service]\n# broker ${label}\n`)
  await put(join(context, "apps/kernel/slice-linux-docker/runtime.txt"), `${label}\n`)
  if (transitionPolicy) {
    await put(
      join(context, "apps/kernel/managed-upgrade-protocol-transitions.json"),
      `${JSON.stringify(transitionPolicy)}\n`,
    )
  }
  const sourceCommit = createHash("sha1").update(`commit-${label}`).digest("hex")
  const sourceTree = createHash("sha1").update(`tree-${label}`).digest("hex")
  if (builderKeys) {
    const diagnostics = "deploy/managed-kernel/runtime-diagnostics.py"
    await put(join(context, diagnostics), await readFile(join(repositoryRoot, diagnostics)))
    for (const sourceFile of [
      "chariox-data-volume-admission.mjs",
      "slice-data-volume-device.mjs",
      "slice-data-volume-protected-io.mjs",
      "slice-disk-quota-xfs-readback.mjs",
    ]) {
      const path = `apps/kernel/slice-linux-docker/${sourceFile}`
      await put(join(context, path), await readFile(join(repositoryRoot, path)))
    }
    const relay = join(context, "apps/kernel/slice-linux-docker/prebuilt/chariox-relay")
    await put(relay, `#!/bin/sh\necho relay-${label}\n`, 0o755)
    const attestationBytes = Buffer.from(JSON.stringify({
      schemaVersion: 1,
      sourceCommit,
      sourceTree,
      target: "x86_64-unknown-linux-gnu",
      artifacts: [
        { name: "chariox-kernel", sha256: await sha256File(kernel) },
        { name: "chariox-managed-bootstrap", sha256: await sha256File(supervisor) },
        { name: "chariox-relay", sha256: await sha256File(relay) },
        ...(includeAppArtifacts ? [
          { name: "chariox-app-package", sha256: await sha256File(appPackage) },
          { name: "chariox-app-storage", sha256: await sha256File(appStorage) },
        ] : []),
      ],
    }))
    await put(attestation, attestationBytes)
    await put(attestationSignature, sign(null, attestationBytes, builderKeys.privateKey).toString("base64"))
    await put(builderKey, rawPublicKey(builderKeys.publicKey).toString("base64"))
  } else {
    await put(attestation, JSON.stringify({ schemaVersion: 1, label }))
    await put(attestationSignature, Buffer.alloc(64, label.charCodeAt(0)).toString("base64"))
    await put(builderKey, Buffer.alloc(32, protocol % 255).toString("base64"))
  }

  const artifactSpecs = [
    ["chariox-kernel", "/usr/local/bin/chariox-kernel", kernel, "file"],
    ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", supervisor, "file"],
    ["chariox-managed-bootstrap.service", `/etc/systemd/system/${serviceName}`, managedService, "file"],
    ["chariox-rootless-docker.service", "/etc/systemd/system/chariox-rootless-docker.service", rootlessService, "file"],
    ["chariox-slice-broker.service", "/etc/systemd/system/chariox-slice-broker.service", brokerService, "file"],
    ["chariox-slice-build-context", "/usr/lib/chariox/slice-build-context", context, "tree"],
    ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", attestation, "file"],
    ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", attestationSignature, "file"],
    ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", builderKey, "file"],
  ]
  if (includeAppArtifacts) {
    artifactSpecs.push(
      ["chariox-app-package", "/usr/local/bin/chariox-app-package", appPackage, "file"],
      ["chariox-app-storage", "/usr/libexec/chariox-app-storage", appStorage, "file"],
      ["chariox-app-storage.service", "/etc/systemd/system/chariox-app-storage.service", appStorageService, "file"],
    )
  }
  const artifacts = []
  if (includeWorkerService) {
    const path = "/etc/systemd/system/chariox-disposable-worker-bootstrap.service"
    const source = join(rootfs, path)
    await put(source, await readFile(join(repositoryRoot, "deploy/managed-kernel/chariox-disposable-worker-bootstrap.service")))
    artifactSpecs.push(["chariox-disposable-worker-bootstrap.service", path, source, "file"])
  }
  if (builderKeys) {
    const path = "/etc/systemd/system/chariox-path1-managed-bootstrap.service"
    const source = join(rootfs, path)
    await put(source, await readFile(join(repositoryRoot, "deploy/managed-kernel/chariox-path1-managed-bootstrap.service")))
    artifactSpecs.push(["chariox-path1-managed-bootstrap.service", path, source, "file"])
    for (const [name, path, source] of [
      ["chariox-data-volume-admission.service", "/etc/systemd/system/chariox-data-volume-admission.service", dataVolumeAdmissionService],
      ["chariox-rootless-docker.path1-data-volume.conf", "/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf", rootlessDataVolumeDropIn],
      ["chariox-slice-disk-quota-allocator.path1-data-volume.conf", "/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf", allocatorDataVolumeDropIn],
    ]) {
      const destination = join(rootfs, path)
      await put(destination, await readFile(source))
      artifactSpecs.push([name, path, destination, "file"])
    }
  }
  for (const [name, path, source, type] of artifactSpecs) {
    artifacts.push({ name, path, sha256: type === "tree" ? await sha256Tree(source) : await sha256File(source) })
  }
  const manifestBytes = Buffer.from(JSON.stringify({
    schemaVersion: manifestSchema,
    ...(manifestSchema === 3 ? { managedUpdateEvidenceVersion: 1 } : {}),
    sourceCommit,
    sourceTree,
    artifacts,
  }))
  await put(join(rootfs, "usr/lib/chariox/release-manifest.json"), manifestBytes)
  await put(
    join(rootfs, "usr/lib/chariox/release-manifest.sig"),
    sign(null, manifestBytes, privateKey).toString("base64"),
  )
  await put(
    join(rootfs, "usr/lib/chariox/release-public-key"),
    rawPublicKey(publicKey).toString("base64"),
  )
  return {
    rootfs,
    digest: `sha256:${createHash("sha256").update(manifestBytes).digest("hex")}`,
  }
}

async function makeHarness(context, {
  currentProtocol = 325,
  targetProtocol = 325,
  currentTransitionPolicy = null,
  targetTransitionPolicy = null,
  receiptKind = "managed_environment",
  workerCapableCurrent = false,
  path1Release = false,
  rotateBuilder = false,
  currentManifestSchema = 3,
  targetManifestSchema = 3,
  currentAppArtifacts = true,
  updaterPath = upgrade,
} = {}) {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-upgrade-"))
  await mkdir(join(root, "tmp"), { mode: 0o700 })
  context.after(() => rm(root, { recursive: true, force: true }))
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const trustedKey = join(root, "trusted-release-public-key")
  await put(trustedKey, rawPublicKey(publicKey).toString("base64"), 0o600)
  const builderKeys = path1Release ? generateKeyPairSync("ed25519") : null
  const trustedBuilderKey = join(root, "trusted-builder-public-key")
  if (builderKeys) await put(trustedBuilderKey, rawPublicKey(builderKeys.publicKey).toString("base64"), 0o600)
  const targetBuilderKeys = rotateBuilder ? generateKeyPairSync("ed25519") : builderKeys
  const nextTrustedBuilderKey = join(root, "next-trusted-builder-public-key")
  if (targetBuilderKeys) await put(nextTrustedBuilderKey, rawPublicKey(targetBuilderKeys.publicKey).toString("base64"), 0o600)
  const current = await makeRelease(
    root, "current", currentProtocol, privateKey, publicKey, currentTransitionPolicy,
    path1Release || workerCapableCurrent || receiptKind === "allocation_worker", builderKeys, currentManifestSchema,
    currentAppArtifacts,
  )
  const target = await makeRelease(
    root, "target", targetProtocol, privateKey, publicKey, targetTransitionPolicy, true, targetBuilderKeys, targetManifestSchema,
  )
  const installRoot = join(root, "host")
  if (path1Release) {
    await mkdir(join(installRoot, "etc/systemd/system/chariox-rootless-docker.service.d"), {
      recursive: true,
      mode: 0o755,
    })
    await mkdir(join(installRoot, "etc/systemd/system/chariox-slice-disk-quota-allocator.service.d"), {
      recursive: true,
      mode: 0o755,
    })
  }
  const releases = join(installRoot, "usr/lib/chariox/releases")
  const currentRelease = join(releases, current.digest.slice("sha256:".length))
  await mkdir(releases, { recursive: true })
  await chmod(installRoot, 0o755)
  await chmod(join(installRoot, "usr"), 0o755)
  await chmod(join(installRoot, "usr/lib"), 0o755)
  await chmod(dirname(releases), 0o755)
  await chmod(releases, 0o755)
  await cp(current.rootfs, currentRelease, { recursive: true, preserveTimestamps: true })
  await symlink(`releases/${current.digest.slice("sha256:".length)}`, join(installRoot, "usr/lib/chariox/current"))
  await symlink("current/usr/lib/chariox/slice-build-context", join(installRoot, "usr/lib/chariox/slice-build-context"))
  if (path1Release) {
    // Match the release-owned host links created by install-image.sh for Path-1.
    for (const [path, target] of [
      ["etc/systemd/system/chariox-data-volume-admission.service",
        "../../../usr/lib/chariox/current/etc/systemd/system/chariox-data-volume-admission.service"],
      ["etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf",
        "../../../../usr/lib/chariox/current/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf"],
      ["etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf",
        "../../../../usr/lib/chariox/current/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf"],
    ]) {
      await symlink(target, join(installRoot, path))
    }
  }
  const receiptPath = join(installRoot, receiptKind === "allocation_worker"
    ? "var/lib/chariox/disposable-worker/bootstrap-receipt.json"
    : "var/lib/chariox/managed/bootstrap-receipt.json")
  const binding = {
    allocationId: "allocation-1",
    expectedHomeKernelId: "home-kernel-1",
    userId: "owner-1",
    realmId: "realm-1",
    workerMachineId: "machine-1",
    workerKernelId: "kernel-1",
    imageDigest: `sha256:${"a".repeat(64)}`,
    runtimeReleaseDigest: current.digest,
    managerOperationId: "operation-1",
    managerOperationFence: 7,
    managerRequestDigest: `sha256:${"b".repeat(64)}`,
    senderKeyThumbprint: `sha256:${"c".repeat(64)}`,
  }
  let bindingDigest = `sha256:${createHash("sha256").update(JSON.stringify(binding)).digest("hex")}`
  const receipt = receiptKind === "allocation_worker" ? {
    schemaVersion: 1, status: "confirmed", allocationId: "allocation-1",
    machineId: "machine-1", kernelId: "kernel-1", relayPublicKey: "relay-public-key",
    runtimeReleaseDigest: current.digest, confirmedAt: "2026-09-11T00:00:00Z",
    homeCaller: { accountId: "account-1", userId: "owner-1", realmId: "realm-1",
      machineId: "home-machine-1", kernelId: "home-kernel-1", relayPublicKey: "home-key" },
  } : receiptKind === "disposable_worker" ? {
    schemaVersion: 1,
    kind: "disposable_worker",
    status: "exchanged",
    cloudApiUrl: "https://cloud.example.test",
    relayPublicKey: "relay-public-key",
    bindingDigest,
    binding,
    enrollmentReceipt: {
      grantId: "grant-1",
      allocationId: binding.allocationId,
      workerMachineId: binding.workerMachineId,
      workerKernelId: binding.workerKernelId,
      imageDigest: binding.imageDigest,
      runtimeReleaseDigest: binding.runtimeReleaseDigest,
      exchangedAt: "2026-09-11T00:00:00Z",
    },
    cloudRelay: {
      apiUrl: "https://cloud.example.test",
      email: "worker@example.test",
      accountId: "account-1",
      userId: binding.userId,
      accountSlug: "account-one",
      realmId: binding.realmId,
      relayUrl: "wss://relay.example.test",
      issuerId: "issuer-1",
      machineId: binding.workerMachineId,
      machineAlias: "Disposable worker",
      machineCredential: `mcred_${"d".repeat(43)}`,
    },
  } : {
    schemaVersion: 1,
    status: "confirmed",
    environmentId: "environment-1",
    machineId: "machine-1",
    kernelId: "kernel-1",
    relayPublicKey: "relay-public-key",
    runtimeReleaseDigest: current.digest,
    confirmedAt: "2026-09-11T00:00:00Z",
    contextPlan: { schemaVersion: 1, repositories: [{ id: "repo-1", url: "ssh://example.invalid/repo" }] },
  }
  if (receiptKind === "allocation_worker") bindingDigest = allocationWorkerBindingDigest(receipt)
  await put(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, 0o640)
  const persistent = {
    credential: join(installRoot, "home/chariox/.chariox/vault/vault.json"),
    provider: join(installRoot, "var/lib/chariox/provider-home/provider-state.json"),
    repository: join(installRoot, "home/chariox/repositories/repo-1/HEAD"),
    workspace: join(installRoot, "home/chariox/workspaces/workspace-1/state"),
    session: join(installRoot, "home/chariox/sessions/session-1.json"),
  }
  for (const [name, path] of Object.entries(persistent)) await put(path, `${name}-sentinel\n`, 0o600)

  const state = join(root, "harness-state")
  const bin = join(root, "bin")
  const charioxIdentity = await effectiveCharioxIdentity()
  // Keep root-owned install fixtures runnable when Node tests execute rootless.
  const rootlessHarness = process.getuid?.() !== 0
  await mkdir(state)
  await mkdir(bin)
  await put(join(bin, "id"), `#!/bin/sh
set -eu
if [ "\${1:-}" = "-u" ] && [ "\${2:-}" = "chariox" ]; then
  printf '%s\n' "${charioxIdentity.uid}"
  exit 0
fi
if [ "\${1:-}" = "-g" ] && [ "\${2:-}" = "chariox" ]; then
  printf '%s\n' "${charioxIdentity.gid}"
  exit 0
fi
if [ "\${MANAGED_UPGRADE_TEST_ROOTLESS:-0}" = 1 ] \
  && [ "\${1:-}" = "-u" ] && [ "$#" -eq 1 ]; then
  printf '0\n'
  exit 0
fi
if [ "\${1:-}" = "-u" ] && [ "\${2:-}" = "chariox-docker" ]; then
  printf '1001\n'
  exit 0
fi
exec /usr/bin/id "$@"
`, 0o755)
  await put(join(bin, "install"), `#!/bin/bash
set -eu
if [ "\${MANAGED_UPGRADE_TEST_ROOTLESS:-0}" != 1 ]; then
  exec /usr/bin/install "$@"
fi
arguments=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|-g) shift 2 ;;
    *) arguments+=("$1"); shift ;;
  esac
done
exec /usr/bin/install "\${arguments[@]}"
`, 0o755)
  await put(join(bin, "systemctl"), `#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$HARNESS_STATE/systemctl.log"
if [ "$1" = disable ] && [ "\${3:-}" = chariox-app-storage.service ]; then
  [ -f "$CHARIOX_MANAGED_UPGRADE_ROOT/etc/systemd/system/chariox-app-storage.service" ] || exit 1
fi
if [ "$1" = "show" ]; then
  case "$*" in
    *--property=NeedDaemonReload*chariox-path1-managed-bootstrap.service)
      if [ -f "$HARNESS_STATE/disk-home-drop-in" ] && [ ! -f "$HARNESS_STATE/systemd-reloaded" ]; then
        printf 'yes\n'
      else
        printf 'no\n'
      fi
      ;;
    *--property=NeedDaemonReload*chariox-disposable-worker-bootstrap.service)
      if [ -f "$HARNESS_STATE/disk-worker-drop-in" ] && [ ! -f "$HARNESS_STATE/systemd-reloaded" ]; then
        printf 'yes\n'
      else
        printf 'no\n'
      fi
      ;;
    *--property=DropInPaths*chariox-path1-managed-bootstrap.service)
      if [ -f "$HARNESS_STATE/campaign-drop-in-path" ]; then
        cat "$HARNESS_STATE/campaign-drop-in-path"
      fi
      if [ -f "$HARNESS_STATE/home-drop-in" ] \
        || { [ -f "$HARNESS_STATE/disk-home-drop-in" ] && [ -f "$HARNESS_STATE/systemd-reloaded" ]; }; then
        printf '%s\n' /etc/systemd/system/chariox-path1-managed-bootstrap.service.d/50-hardening.conf
      fi
      ;;
    *--property=DropInPaths*chariox-disposable-worker-bootstrap.service)
      if [ -f "$HARNESS_STATE/worker-drop-in" ] \
        || { [ -f "$HARNESS_STATE/disk-worker-drop-in" ] && [ -f "$HARNESS_STATE/systemd-reloaded" ]; }; then
        printf '%s\n' /etc/systemd/system/chariox-disposable-worker-bootstrap.service.d/50-hardening.conf
      fi
      ;;
  esac
  exit 0
fi
if [ "$1" = "daemon-reload" ]; then
  if [ -f "$HARNESS_STATE/disk-home-drop-in" ] || [ -f "$HARNESS_STATE/disk-worker-drop-in" ]; then
    printf 'present\n' > "$HARNESS_STATE/systemd-reloaded"
  fi
  if [ -f "$HARNESS_STATE/worker-drop-in-after-reload" ]; then
    rm -f -- "$HARNESS_STATE/worker-drop-in-after-reload"
    printf 'present\n' > "$HARNESS_STATE/worker-drop-in"
  fi
fi
presence="$CHARIOX_MANAGED_UPGRADE_ROOT/home/chariox/.chariox/kernels/active/kernel-1.json"
if [ "$1" = "stop" ]; then
  case "\${2:-}" in
    chariox-managed-bootstrap.service|chariox-path1-managed-bootstrap.service|chariox-disposable-worker-bootstrap.service)
  if [ -f "$HARNESS_STATE/write-legacy-home-on-stop" ]; then
    legacy_home="$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/home"
    mkdir -p "$legacy_home"
    printf 'late-service-write\n' > "$legacy_home/late-service-write"
    rm -f -- "$HARNESS_STATE/write-legacy-home-on-stop"
  fi
  rm -f -- "$presence"
      ;;
  esac
fi
if [ "$1" = "start" ] && [ "$2" = "chariox-rootless-docker.service" ] \
  && [ "\${CHARIOX_MANAGED_PROVIDER_TOPOLOGY:-}" = path1 ]; then
  touch "$HARNESS_STATE/data-volume-mounted"
fi
case "\${2:-}" in
  chariox-managed-bootstrap.service|chariox-path1-managed-bootstrap.service|chariox-disposable-worker-bootstrap.service)
if [ "$1" = "start" ]; then
  if [ -f "$HARNESS_STATE/rebind-grant-once" ]; then
    rm -f -- "$HARNESS_STATE/rebind-grant-once"
    printf '{"schemaVersion":2}\n' > "$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/managed/bootstrap-grant-binding.json"
  fi
  if [ -f "$HARNESS_STATE/check-builder-pin-on-start" ]; then
    cmp -s "$CHARIOX_MANAGED_UPGRADE_ROOT/etc/chariox/trusted-builder-public-key" \
      "$CHARIOX_MANAGED_UPGRADE_ROOT/usr/lib/chariox/current/usr/lib/chariox/builder-public-key" || exit 1
    printf 'matched\n' >> "$HARNESS_STATE/builder-pin-starts"
  fi
  if [ -f "$HARNESS_STATE/skip-presence-once" ]; then
    rm -f -- "$HARNESS_STATE/skip-presence-once"
  else
    mkdir -p "\${presence%/*}"
    protocol=$("$CHARIOX_MANAGED_UPGRADE_ROOT/usr/lib/chariox/current/usr/local/bin/chariox-kernel" --print-local-daemon-protocol-version)
    now=$(node -e 'process.stdout.write(String(Date.now()))')
    machine_id=machine-1
    if [ -f "$HARNESS_STATE/wrong-machine-once" ]; then
      rm -f -- "$HARNESS_STATE/wrong-machine-once"
      machine_id=machine-wrong
    fi
    if [ -f "$HARNESS_STATE/stale-presence-once" ]; then
      rm -f -- "$HARNESS_STATE/stale-presence-once"
      now=1
    fi
    if [ -f "$HARNESS_STATE/oversized-presence-once" ]; then
      rm -f -- "$HARNESS_STATE/oversized-presence-once"
      node -e 'process.stdout.write("x".repeat(32769))' > "$presence"
    else
      printf '{"schema_version":1,"kernel_id":"kernel-1","machine_id":"%s","host":"127.0.0.1","port":%s,"relay_connected":true,"process_id":%s,"started_at_ms":%s,"heartbeat_at_ms":%s,"local_daemon_protocol_version":%s}\n' \
        "$machine_id" "$CHARIOX_MANAGED_UPGRADE_HEALTH_PORT" "$PPID" "$now" "$now" "$protocol" > "$presence"
    fi
    if [ -f "$HARNESS_STATE/wrong-release-once" ]; then
      rm -f -- "$HARNESS_STATE/wrong-release-once"
      receipt="$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/managed/bootstrap-receipt.json"
      sed 's/sha256:[a-f0-9]\\{64\\}/sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff/' \
        "$receipt" > "$receipt.wrong-release"
      chmod 0640 "$receipt.wrong-release"
      mv "$receipt.wrong-release" "$receipt"
    fi
    if [ -f "$HARNESS_STATE/wrong-environment-once" ]; then
      rm -f -- "$HARNESS_STATE/wrong-environment-once"
      receipt="$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/managed/bootstrap-receipt.json"
      sed 's/"environmentId": "environment-1"/"environmentId": "environment-wrong"/' \
        "$receipt" > "$receipt.wrong-environment"
      chmod 0640 "$receipt.wrong-environment"
      mv "$receipt.wrong-environment" "$receipt"
    fi
  fi
fi
    ;;
esac
if [ "$1" = "start" ] \
  && [ -f "$HARNESS_STATE/crash-after-supervisor-start" ]; then
  case "\${2:-}" in
    chariox-managed-bootstrap.service|chariox-path1-managed-bootstrap.service|chariox-disposable-worker-bootstrap.service)
      rm -f -- "$HARNESS_STATE/crash-after-supervisor-start"
      case "\${PPID:-}" in ''|0|1|*[!0-9]*) exit 1 ;; esac
      kill -KILL "$PPID"
      exit 1
      ;;
  esac
fi
if [ "$1" = "is-active" ]; then
  case "\${3:-}" in
    chariox-managed-bootstrap.service|chariox-path1-managed-bootstrap.service|chariox-disposable-worker-bootstrap.service)
      if [ -f "$HARNESS_STATE/fail-health-once" ]; then
        rm -f "$HARNESS_STATE/fail-health-once"
        exit 1
      fi
      if [ -f "$HARNESS_STATE/fail-health-always" ]; then
        exit 1
      fi
      ;;
  esac
fi
exit 0
`, 0o755)
  await put(join(bin, "mountpoint"), `#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$HARNESS_STATE/mountpoint.log"
[ "\${1:-}" = "--quiet" ] && [ "\${2:-}" = "/var/lib/chariox-docker/data" ] \
  && [ -f "$HARNESS_STATE/data-volume-mounted" ]
`, 0o755)
  await put(join(bin, "node"), `#!/bin/sh
set -eu
# Every crash target is the direct updater parent started by this fixture.
crash_updater() {
  case "\${PPID:-}" in ''|0|1|*[!0-9]*) exit 1 ;; esac
  kill -KILL "$PPID"
}
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \\
  && [ "\${2:-}" = "atomic-text" ] \\
  && [ "\${3:-}" = stopped ] \\
  && [ -f "$HARNESS_STATE/fail-after-stopped" ]; then
  "${process.execPath}" "$@"
  rm -f "$HARNESS_STATE/fail-after-stopped"
  exit 23
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \\
  && [ "\${2:-}" = "sync-directory" ] \\
  && [ "\${3:-}" = "$CHARIOX_MANAGED_UPGRADE_ROOT/usr/lib/chariox/releases" ] \\
  && [ -f "$HARNESS_STATE/crash-after-release-publication" ]; then
  "${process.execPath}" "$@"
  rm -f "$HARNESS_STATE/crash-after-release-publication"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \\
  && [ "\${2:-}" = "atomic-file" ] \\
  && [ "\${4##*/}" = "trusted-builder-public-key" ] \\
  && [ -f "$HARNESS_STATE/crash-after-builder-pin" ]; then
  "${process.execPath}" "$@"
  rm -f "$HARNESS_STATE/crash-after-builder-pin"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "validate-receipt-match" ] \
  && [ -f "$HARNESS_STATE/fail-final-receipt-validation" ]; then
  rm -f -- "$HARNESS_STATE/fail-final-receipt-validation"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-home-migration.mjs" ] \
  && [ "\${2:-}" = "apply" ] \
  && [ -f "$HARNESS_STATE/crash-after-home-root-rename" ]; then
  /bin/mv -- "\${5}" "\${6}"
  rm -f -- "$HARNESS_STATE/crash-after-home-root-rename"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-home-migration.mjs" ] \
  && [ "\${2:-}" = "apply" ] \
  && [ -f "$HARNESS_STATE/crash-before-home-completion-marker" ]; then
  "${process.execPath}" "$@"
  rm -f -- "\${3%/*}/home-migration-complete" \
    "$HARNESS_STATE/crash-before-home-completion-marker"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-symlink" ] \
  && [ "\${4##*/}" = current ] \
  && [ -f "$HARNESS_STATE/crash-after-pre-apps-current" ]; then
  "${process.execPath}" "$@"
  if [ ! -f "$CHARIOX_MANAGED_UPGRADE_ROOT/usr/lib/chariox/current/usr/libexec/chariox-app-storage" ]; then
    rm -f "$HARNESS_STATE/crash-after-pre-apps-current"
    crash_updater
    exit 1
  fi
  exit 0
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-symlink" ] \
  && [ -f "$HARNESS_STATE/crash-before-symlink" ]; then
  rm -f "$HARNESS_STATE/crash-before-symlink"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-symlink" ] \
  && [ "\${4##*/}" = "slice-build-context" ] \
  && [ -f "$HARNESS_STATE/crash-after-facade-symlink" ]; then
  rm -f "$HARNESS_STATE/crash-after-facade-symlink"
  "${process.execPath}" "$@"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-text" ] \
  && [ -f "$HARNESS_STATE/crash-after-phase-\${3:-}" ]; then
  "${process.execPath}" "$@"
  rm -f "$HARNESS_STATE/crash-after-phase-\${3:-}"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "publish-transaction" ] \
  && [ -f "$HARNESS_STATE/crash-after-phase-prepared" ]; then
  "${process.execPath}" "$@"
  rm -f "$HARNESS_STATE/crash-after-phase-prepared"
  crash_updater
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "tombstone-transaction" ]; then
  terminal_phase=$(sed -n '1p' "\${3:-}/phase")
  marker="$HARNESS_STATE/crash-after-\${terminal_phase}-tombstone"
  if [ -f "$marker" ]; then
    "${process.execPath}" "$@"
    rm -f "$marker"
    crash_updater
    exit 1
  fi
fi
exec "${process.execPath}" "$@"
`, 0o755)
  await put(join(bin, "stat"), `#!/bin/sh
set -eu
if [ "\${1:-}" = "-c" ] && [ "\${2:-}" = "%u" ]; then
  if [ -f "$HARNESS_STATE/non-root-key-owner" ] && [ "\${3:-}" = "$MANAGED_TRUSTED_KEY" ]; then
    printf '1\n'
    exit 0
  fi
  if [ -f "$HARNESS_STATE/foreign-receipt-ancestor" ] && [ "\${3:-}" = "$MANAGED_RECEIPT_DIRECTORY" ]; then
    printf '1\n'
    exit 0
  fi
  if [ "\${MANAGED_UPGRADE_TEST_ROOTLESS:-0}" = 1 ]; then
    printf '0\n'
    exit 0
  fi
fi
exec /usr/bin/stat "$@"
`, 0o755)
  const server = createServer()
  await new Promise((resolvePromise, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolvePromise)
  })
  context.after(() => new Promise((resolvePromise) => server.close(resolvePromise)))
  const port = server.address().port
  const env = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    MANAGED_UPGRADE_TEST_ROOTLESS: rootlessHarness ? "1" : "0",
    HARNESS_STATE: state,
    TMPDIR: join(root, "tmp"),
    MANAGED_TRUSTED_KEY: trustedKey,
    MANAGED_RECEIPT_DIRECTORY: dirname(receiptPath),
    CHARIOX_MANAGED_UPGRADE_ROOT: installRoot,
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "shared_host",
    CHARIOX_MANAGED_UPGRADE_LOCK: join(state, "upgrade.lock"),
    CHARIOX_MANAGED_UPGRADE_HEALTH_HOST: "127.0.0.1",
    CHARIOX_MANAGED_UPGRADE_HEALTH_PORT: String(port),
    CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS: "1000",
  }
  const run = (extraEnv = {}, args = [target.rootfs, current.digest, target.digest, trustedKey]) =>
    spawnSync(updaterPath, args, { encoding: "utf8", env: { ...env, ...extraEnv } })
  return {
    root, installRoot, receiptPath, receipt, bindingDigest, persistent, charioxIdentity,
    current, target, trustedKey, trustedBuilderKey, nextTrustedBuilderKey, state, env, run,
  }
}

async function installManagedHotpatchFacade(harness) {
  const facade = join(harness.installRoot, "usr/lib/chariox/slice-build-context")
  const hotpatch = join(harness.root, "usr/local/lib/chariox/managed-hotpatch/slice-build-context-6694e438d1")
  await mkdir(hotpatch, { recursive: true })
  await rm(facade)
  await symlink(hotpatch, facade)
  return { facade, hotpatch }
}

async function persistentSnapshot(paths) {
  return Promise.all(Object.entries(paths).map(async ([name, path]) => ({
    name,
    contents: await readFile(path, "utf8"),
    mode: (await stat(path)).mode & 0o777,
  })))
}

async function treeSnapshot(root) {
  const entries = []
  async function visit(directory, prefix = "") {
    const names = (await readdir(directory)).sort()
    for (const name of names) {
      const path = join(directory, name)
      const relativePath = prefix ? `${prefix}/${name}` : name
      const metadata = await lstat(path)
      const entry = {
        path: relativePath,
        mode: metadata.mode & 0o777,
        mtimeMs: metadata.mtimeMs,
      }
      if (metadata.isSymbolicLink()) {
        entries.push({ ...entry, type: "symlink", target: await readlink(path) })
      } else if (metadata.isDirectory()) {
        entries.push({ ...entry, type: "directory" })
        await visit(path, relativePath)
      } else {
        entries.push({ ...entry, type: "file", contents: await readFile(path) })
      }
    }
  }
  await visit(root)
  return entries
}


export {
  rawPublicKey, makeRelease, put, createRootPrivateDirectory, makeHarness,
  installManagedHotpatchFacade, persistentSnapshot, treeSnapshot,
}
