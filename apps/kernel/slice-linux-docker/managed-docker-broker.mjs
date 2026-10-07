#!/usr/bin/env node

import { retireProtectedQuotaHomes } from "./protected-home-retirement.mjs"
import { recordCapturedImageProof, recordFlattenedImageProof } from "./protected-image-proof.mjs"
import { captureNeedsFlatten, flattenedImageId } from "./captured-image-depth.mjs"
import { requireSafeHomeVolume as requireSafeManagedHomeVolume } from "./protected-home-preflight.mjs"
import { requireSafeLocalHomeVolume } from "./protected-local-home-scan.mjs"
import { createManagedLayoutController } from "./protected-managed-layout.mjs"
import { LEGACY_CAPTURE_NOTICE } from "./legacy-release-f-layout.mjs"
import { localDevRuntimeEnvironment } from "./protected-local-docker-authority.mjs"
import { verifiedProtectedAuthority } from "./protected-authority.mjs"
import { recordManagedImageProof } from "./protected-image-proof.mjs"
import { DURABLE_LAYOUT_ROOT } from "./protected-first-boot.mjs"
import { dockerControlPolicy, runBrokerCommand } from "./managed-broker-command.mjs"

import { spawnSync } from "node:child_process"
import { isPositiveDockerCpuLimit } from "./docker-cpu-policy.mjs"
import { isDockerImageReference } from "./docker-image-reference.mjs"
import { HOME_ARCHIVE_PROGRESS_TIMEOUT_MS, homeArchiveMetadataMatches } from "./managed-home-archive-stream.mjs"
import { createHash } from "node:crypto"
import { digestPinnedHomeArchive } from "./managed-home-archive-digest.mjs"
import {
  chmodSync,
  chownSync,
  closeSync,
  constants,
  existsSync,
  fstatSync,
  fsyncSync,
  lstatSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs"
import { createServer } from "node:net"
import { createInterface } from "node:readline"
import { basename, dirname, isAbsolute, join, resolve, sep } from "node:path"
import { fileURLToPath } from "node:url"
import { requestSliceDiskQuota, sliceDiskQuotaIdentityFromEnvironment } from "./slice-disk-quota-client.mjs"
import { validateSliceDiskQuotaIdentity } from "./slice-disk-quota-contract.mjs"
import { createSliceDiskQuotaCoordinator } from "./slice-disk-quota-coordinator.mjs"
import {
  dockerObjectNotFound,
  readSliceDiskQuotaMarkerInspection,
  runWithSliceDiskQuotaAdmission,
} from "./slice-disk-quota-admission.mjs"

const MAX_FRAME_BYTES = 12 * 1024 * 1024
const MAX_OUTPUT_BYTES = 64 * 1024
const MAX_CREDENTIAL_BYTES = 2 * 1024 * 1024
const MAX_CREDENTIAL_TOTAL_BYTES = 8 * 1024 * 1024
const LINUX_O_PATH = 0x200000
const PROVIDER_ACCOUNT_CREDENTIAL_PATH = /^\/home\/slice\/\.chariox\/daemon\/provider-accounts\/[A-Za-z0-9-]+\/(?:codex\/[A-Za-z0-9-]+\/codex\/auth\.json|opencode\/[A-Za-z0-9-]+\/data\/opencode\/auth\.json)$/
const PROTECTED_PROVIDER_ACCOUNT_CREDENTIAL_PATH = /^\/var\/lib\/chariox\/slice-private\/provider-accounts\/[A-Za-z0-9-]+\/(?:codex\/[A-Za-z0-9-]+\/codex\/auth\.json|opencode\/[A-Za-z0-9-]+\/data\/opencode\/auth\.json)$/
const SHARE_ROOT_INPUT = resolve(process.env.CHARIOX_SLICE_DOCKER_SHARE_ROOT ?? "/var/lib/chariox-slice-share")
const SHARE_ROOT = existsSync(SHARE_ROOT_INPUT) ? realpathSync(SHARE_ROOT_INPUT) : SHARE_ROOT_INPUT
const SOCKET_PATH = process.env.CHARIOX_SLICE_DOCKER_BROKER_SOCKET ?? "/var/lib/chariox-slice-share/.broker-private/control/control.sock"
const DOCKER_HOST = process.env.DOCKER_HOST ?? "unix:///run/chariox-docker/docker.sock"
const BROKER_INPUT_ROOT = resolve(
  process.env.CHARIOX_SLICE_DOCKER_BROKER_INPUT_ROOT ?? "/run/chariox-slice-broker/input",
)
const BROKER_OUTPUT_ROOT = resolve(
  process.env.CHARIOX_SLICE_DOCKER_BROKER_OUTPUT_ROOT ?? join(SHARE_ROOT, ".broker-private/output"),
)
const BROKER_ARTIFACT_ROOT = resolve(
  process.env.CHARIOX_SLICE_DOCKER_BROKER_ARTIFACT_ROOT ?? "/var/lib/chariox-slice-share/.broker-private/artifacts",
)
const HANDLE_ROOT = resolve(process.env.CHARIOX_SLICE_DOCKER_HANDLE_ROOT ?? "/var/lib/chariox-docker/mount-handles")
const HANDLE_STATE = resolve(process.env.CHARIOX_SLICE_DOCKER_HANDLE_STATE ?? "/var/lib/chariox-docker/mount-handles.json")
const sliceDiskQuotaCoordinator = createSliceDiskQuotaCoordinator()
const MAX_PERSISTENT_HANDLES = 256
const PROVISIONER = resolve(
  process.env.CHARIOX_SLICE_DOCKER_PROVISIONER ??
    resolve(dirname(fileURLToPath(import.meta.url)), "provision-linux-docker-slice.sh"),
)
const RELEASE_MANIFEST = process.env.CHARIOX_MANAGED_RELEASE_MANIFEST

function signedBuildContextDigest() {
  if (!RELEASE_MANIFEST) return undefined
  const manifest = JSON.parse(readFileSync(RELEASE_MANIFEST, "utf8"))
  const artifact = manifest.artifacts?.find(
    (candidate) =>
      candidate?.name === "chariox-slice-build-context" &&
      candidate?.path === "/usr/lib/chariox/slice-build-context",
  )
  if (!artifact || !/^sha256:[a-f0-9]{64}$/.test(artifact.sha256)) {
    fail("signed release does not identify the slice build context")
  }
  return artifact.sha256
}

const LOCAL_AUTHORITY = process.env.CHARIOX_SLICE_LOCAL_DEV_OWNER_UID === undefined
  ? undefined : verifiedProtectedAuthority()
const VERIFIED_BUILD_CONTEXT_DIGEST = LOCAL_AUTHORITY?.sourceDigest ?? signedBuildContextDigest()
const protectedLayouts = createManagedLayoutController({
  root: DURABLE_LAYOUT_ROOT, sourceDigest: process.platform === "linux" && process.getuid() === 0
    && (LOCAL_AUTHORITY ? DOCKER_HOST === "unix:///run/docker.sock"
      : DOCKER_HOST === "unix:///run/chariox-docker/docker.sock") ? VERIFIED_BUILD_CONTEXT_DIGEST : undefined,
  dataOwner: () => {
    const authority = verifiedProtectedAuthority()
    if (DOCKER_HOST !== (authority.kind === "linux-local-rootful-dev"
        ? "unix:///run/docker.sock" : "unix:///run/chariox-docker/docker.sock")) fail("protected slice Docker authority mismatch")
    return authority.dataUid
  },
  docker: args => spawnSync("/usr/bin/docker", args, {
    env: dockerEnvironment(), encoding: "utf8", maxBuffer: 1024 * 1024, timeout: 60_000,
  }),
})
const persistentHandleDescriptors = new Map()
let persistentHandleRecords
const ACTIONS = new Set([
  "provision",
  "restore-state",
  "recover",
  "import-provider-auth",
  "remove-provider-auth",
  "stop",
  "destroy",
  "start-provider-login",
])
const PATH_ENVIRONMENT = new Set([
  "CHARIOX_SLICE_WORKSPACE",
  "CHARIOX_SLICE_SAVED_HOME_ARCHIVE",
])
const CREDENTIAL_ENVIRONMENT = new Set([
  "CHARIOX_SLICE_GITHUB_TOKEN_FILE",
])
const ROOM_BINDING_ENVIRONMENT = [
  "CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID",
  "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY",
  "CHARIOX_ROOM_ENVIRONMENT_SESSION_ID",
  "CHARIOX_ROOM_ENVIRONMENT_SLICE_ID",
]
const ALLOWED_ENVIRONMENT = new Set([
  ...ROOM_BINDING_ENVIRONMENT,
  "CHARIOX_SLICE_NAME",
  "CHARIOX_SLICE_HOSTNAME",
  "CHARIOX_SLICE_ID",
  "CHARIOX_SLICE_OWNER_KERNEL_ID",
  "CHARIOX_SLICE_OWNER_MACHINE_ID",
  "CHARIOX_SLICE_OWNER_PUBLIC_KEY",
  "CHARIOX_SLICE_DOCKER_PIDS_LIMIT",
  "CHARIOX_SLICE_DOCKER_NOFILE_LIMIT",
  "CHARIOX_SLICE_MIN_FREE_MB",
  "CHARIOX_SLICE_DOCKER_IMAGE",
  "CHARIOX_SLICE_BASE_IMAGE",
  "CHARIOX_SLICE_BUILD_IMAGE",
  "CHARIOX_SLICE_HOME_VOLUME",
  "CHARIOX_SLICE_SCREEN_GEOMETRY",
  "CHARIOX_SLICE_CODEX_PORT",
  "CHARIOX_SLICE_OPENCODE_PORT",
  "CHARIOX_SLICE_CODEX_PORT_RANGE",
  "CHARIOX_SLICE_OPENCODE_PORT_RANGE",
  "CHARIOX_SLICE_KERNEL_PORT",
  "CHARIOX_SLICE_MCP_PORT",
  "CHARIOX_SLICE_RELAY_PORT",
  "CHARIOX_SLICE_NOVNC_PORT",
  "CHARIOX_SLICE_VIEWER_BACKEND",
  "CHARIOX_SLICE_DISPLAY_MODE",
  "CHARIOX_SLICE_START_DESKTOP",
  "CHARIOX_SLICE_START_PROVIDER_SERVERS",
  "CHARIOX_SLICE_START_RUNTIME",
  "CHARIOX_SLICE_IMPORT_PROVIDER_AUTH",
  "CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE",
  "CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP",
  "CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY",
  "CHARIOX_SLICE_APPARMOR_PROFILE",
  "CHARIOX_SLICE_PROVIDER_BIND_HOST",
  "CHARIOX_SLICE_DAEMON_ID",
  "CHARIOX_SLICE_DAEMON_ALIAS",
  "CHARIOX_SLICE_MACHINE_ID",
  "CHARIOX_SLICE_MACHINE_ALIAS",
  "CHARIOX_SLICE_DOCKER_MEMORY",
  "CHARIOX_SLICE_DOCKER_CPUS",
  "CHARIOX_SLICE_DISK_LAYER_MB",
  "CHARIOX_SLICE_DISK_HOME_MB",
  "CHARIOX_SLICE_RELAY_TOKEN",
  "CHARIOX_SLICE_RELAY_URL",
  "CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT",
  "CHARIOX_SLICE_WORKSPACE_MOUNT_MODE",
  "CHARIOX_SLICE_ACCOUNT_OWNER",
  "CHARIOX_SLICE_ACCOUNT_PROFILE",
  "CHARIOX_SLICE_AUTH_PROVIDER",
  "CHARIOX_SLICE_LOGIN_PROVIDER",
  ...PATH_ENVIRONMENT,
])

function fail(message) {
  throw new Error(message)
}

function exactKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(`${label} must be an object`)
  const actual = Object.keys(value).sort()
  const wanted = [...expected].sort()
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    fail(`${label} contains unsupported fields`)
  }
}

function isUnderShare(path) {
  if (!isAbsolute(path)) return false
  const canonical = resolve(path)
  return canonical === SHARE_ROOT || canonical.startsWith(`${SHARE_ROOT}${sep}`)
}

function validateSharedPath(path, label) {
  const candidate = resolve(path)
  if (
    path.includes("\0") ||
    (candidate !== SHARE_ROOT_INPUT && !candidate.startsWith(`${SHARE_ROOT_INPUT}${sep}`))
  ) {
    fail(`${label} must stay under the managed slice share`)
  }
  let existing = resolve(path)
  while (!existsSync(existing)) {
    const parent = dirname(existing)
    if (parent === existing) fail(`${label} has no existing managed parent`)
    existing = parent
  }
  const canonical = realpathSync(existing)
  if (!isUnderShare(canonical)) fail(`${label} resolves outside the managed slice share`)
  let current = SHARE_ROOT_INPUT
  const relative = candidate.slice(SHARE_ROOT_INPUT.length).split(sep).filter(Boolean)
  for (const component of relative) {
    current = resolve(current, component)
    if (!existsSync(current)) break
    if (lstatSync(current).isSymbolicLink()) fail(`${label} contains a symbolic link`)
  }
}

function validateResource(value, label) {
  if (typeof value !== "string" || !/^chariox-[a-zA-Z0-9_.:-]{1,180}$/.test(value)) {
    fail(`${label} must be a Chariox-owned Docker resource`)
  }
}

function validateSliceContainer(value, label) {
  validateResource(value, label)
  if (!value.startsWith("chariox-slice-")) fail(`${label} is not a managed slice resource`)
}

function isDiskAdmissionHelper(value) {
  return /-disk-admission-[a-f0-9]{16}$/.test(value)
}

const SLICE_RUNTIME_LOG_SCRIPT = `
set -eu
found=0
case "$2" in
  protected) runtime=/var/lib/chariox/slice-private/runtime/logs; kernel=/var/lib/chariox/slice-private/kernel/logs ;;
  legacy) runtime=/opt/chariox-slice/logs; kernel=/home/slice/.local/state/chariox/logs ;;
  *) exit 64 ;;
esac
for file in "$runtime"/*.log "$kernel"/*.ndjson; do
  [ -f "$file" ] || continue
  found=1
  printf '\\n=== %s ===\\n' "$file"
  tail -n "$1" "$file"
done
if [ "$found" -eq 0 ]; then
  printf '<no slice runtime logs>\\n'
fi
`

const SCREEN_ACTIONS = new Set(["start", "stop", "status", "prepare", "interact"])
const VIEWER_BACKENDS = new Set(["novnc", "selkies"])

function validateConfiguredScreenExec(args) {
  if (
    args.length !== 12 || args[3] !== "-e" || args[5] !== "-e" ||
    args[7] !== "-u" || args[8] !== "slice" ||
    args[10] !== "/opt/chariox-slice/slice-screen.sh" || !SCREEN_ACTIONS.has(args[11])
  ) fail("Docker screen command shape is invalid")
  const fields = ["CHARIOX_SLICE_VIEWER_BACKEND", "CHARIOX_SLICE_NOVNC_PORT", "CHARIOX_SLICE_DISPLAY_MODE"]
  const values = fields.map((name, index) => {
    const argument = args[2 + index * 2]
    if (!argument.startsWith(`${name}=`)) fail("Docker screen environment field is invalid")
    return argument.slice(name.length + 1)
  })
  const [backend, port, mode] = values
  if (
    !VIEWER_BACKENDS.has(backend) || !["headed", "headless"].includes(mode) ||
    port.length < 1 || port.length > 5 || /[^0-9]/.test(port) || Number(port) > 65535
  ) fail("Docker screen environment value is invalid")
  validateSliceContainer(args[9], "Docker screen container")
}

function validateDockerExec(args) {
  if (args[1] === "-e") {
    validateConfiguredScreenExec(args)
    return
  }
  if (args[1] !== "-u" || !["slice", "root"].includes(args[2])) fail("Docker exec user is invalid")
  validateSliceContainer(args[3], "Docker exec container")
  const command = args.slice(4)
  if (
    args[2] === "slice" &&
    command.length === 3 &&
    exactArguments(command.slice(0, 2), ["test", "-s"]) &&
    (PROVIDER_ACCOUNT_CREDENTIAL_PATH.test(command[2]) || PROTECTED_PROVIDER_ACCOUNT_CREDENTIAL_PATH.test(command[2]))
  ) return
  if (args[2] === "slice" && exactArguments(command, ["gh", "auth", "token", "--hostname", "github.com"])) return
  if (
    args[2] === "slice" &&
    exactArguments(command, [
      "jq",
      "-r",
      ".relay_url",
      "/home/slice/.chariox/daemon/config.json",
    ])
  ) return
  if (
    args[2] === "slice" &&
    command.length === 2 &&
    command[0] === "/opt/chariox-slice/slice-screen.sh" &&
    SCREEN_ACTIONS.has(command[1])
  ) return
  if (
    args[2] === "root" &&
    isDiskAdmissionHelper(args[3]) &&
    exactArguments(command, ["du", "-sb", "/home-src"])
  ) return
  if (
    args[2] === "root" &&
    isDiskAdmissionHelper(args[3]) &&
    exactArguments(command, ["bash", "-lc", "set -euo pipefail; find /home-src -printf . | wc -c"])
  ) return
  if (
    args[2] === "root" &&
    isDiskAdmissionHelper(args[3]) &&
    exactArguments(command, ["df", "-B1", "--output=avail", "/tmp"])
  ) return
  if (
    args[2] === "slice" &&
    command.length === 3 &&
    command[0] === "bash" &&
    command[1] === "-lc" &&
    command[2] === "screen -S chariox-slice-relay -X quit >/dev/null 2>&1 || true; screen -S chariox-slice-kernel -X quit >/dev/null 2>&1 || true; pkill -f 'codex app-server' >/dev/null 2>&1 || true; pkill -f 'opencode serve' >/dev/null 2>&1 || true"
  ) return
  if (
    args[2] === "slice" &&
    command.length === 6 &&
    command[0] === "sh" &&
    command[1] === "-c" &&
    command[2] === SLICE_RUNTIME_LOG_SCRIPT &&
    command[3] === "slice-runtime-logs" &&
    /^[0-9]{1,4}$/.test(command[4]) &&
    new Set(["legacy", "protected"]).has(command[5])
  ) return
  fail("Docker exec command shape is not allowed")
}

function exactArguments(args, expected) {
  return args.length === expected.length && args.every((argument, index) => argument === expected[index])
}

function validateDocker(args) {
  if (!Array.isArray(args) || args.length === 0 || args.length > 64) fail("Docker arguments are invalid")
  for (const arg of args) {
    if (typeof arg !== "string" || arg.length > 16 * 1024 || arg.includes("\0")) fail("Docker argument is invalid")
  }
  if (
    exactArguments(args, ["info"])
    || exactArguments(args, ["info", "--format", "{{.MemTotal}}"])
    || exactArguments(args, ["info", "--format", "{{.DockerRootDir}}"])
    || exactArguments(args, ["ps", "--format", "{{.Names}}"])
    || exactArguments(args, ["ps", "-a", "--format", "{{.Names}}"])
  ) return
  if (args[0] === "inspect" && args.length === 4 && args[1] === "--format") {
    if (!["{{.State.Running}} {{.State.Status}}", "{{.HostConfig.Memory}}", '{{index .Config.Labels "io.chariox.snapshot-helper"}}', "{{.Image}}"].includes(args[2])) {
      fail("Docker inspect format is invalid")
    }
    validateSliceContainer(args[3], "Docker container")
    return
  }
  if (
    args[0] === "inspect" &&
    args.length === 5 &&
    (exactArguments(args.slice(1, 4), ["--size", "--format", "{{.SizeRw}}"])
      || exactArguments(args.slice(1, 4), ["--size", "--format", "{{.SizeRootFs}}"]))
  ) {
    validateSliceContainer(args[4], "Docker container")
    return
  }
  if (args[0] === "logs" && args.length === 4 && args[1] === "--tail" && /^[0-9]{1,4}$/.test(args[2])) {
    validateSliceContainer(args[3], "Docker container")
    return
  }
  if (args[0] === "exec" && args.length >= 5) {
    validateDockerExec(args)
    return
  }
  if (exactArguments(args.slice(0, 3), ["image", "rm", "-f"]) && args.length === 4) {
    validateResource(args[3], "Docker image")
    return
  }
  if ((exactArguments(args.slice(0, 4), ["image", "inspect", "--format", "{{.Id}}"])
      || exactArguments(args.slice(0, 4), ["image", "inspect", "--format", "{{len .RootFS.Layers}}"]))
      && args.length === 5) {
    if (!isDockerImageReference(args[4])) fail("Docker image reference is invalid")
    return
  }
  if (exactArguments(args.slice(0, 2), ["container", "inspect"]) && args.length === 3) {
    validateSliceContainer(args[2], "Docker container")
    return
  }
  if (["start", "stop", "pause", "unpause"].includes(args[0]) && args.length === 2) {
    validateSliceContainer(args[1], "Docker container")
    return
  }
  if (args[0] === "rm" && args.length === 3 && args[1] === "-f") {
    validateSliceContainer(args[2], "Docker container")
    return
  }
  if (args[0] === "commit" && args.length === 3) {
    validateSliceContainer(args[1], "Docker container")
    validateResource(args[2], "Docker image")
    return
  }
  if (
    args[0] === "create" &&
    args.length === 20 &&
    args[1] === "--name" &&
    exactArguments(args.slice(3, 11), ["--memory", "512m", "--cpus", "1", "--pids-limit", "64", "--network", "none"]) &&
    args[11] === "--label" && args[12] === `io.chariox.snapshot-helper=${args[2]}` &&
    args[13] === "--user" &&
    args[14] === "root" &&
    args[15] === "-v" &&
    args[18] === "sleep" &&
    args[19] === "infinity"
  ) {
    validateSliceContainer(args[2], "Docker helper container")
    const suffix = /-(?:disk-admission-[a-f0-9]{16}|home-archive-[0-9]{1,20})$/.exec(args[2])
    if (!suffix) fail("Docker helper identity is invalid")
    const [volume, target, extra] = args[16].split(":")
    validateResource(volume, "Docker volume")
    if (volume !== `${args[2].slice(0, suffix.index)}-home`) fail("Docker helper volume does not match its slice")
    if (target !== "/home-src" || extra !== "ro") fail("Docker helper volume target is invalid")
    if (!isDockerImageReference(args[17])) fail("Docker helper image reference is invalid")
    return
  }
  if (args[0] === "cp" && args.length === 3) {
    const operands = args.slice(1).map((operand) => {
      const separator = operand.indexOf(":")
      if (separator < 0) return { kind: "host", path: operand }
      validateSliceContainer(operand.slice(0, separator), "Docker copy container")
      if (!operand.slice(separator + 1).startsWith("/")) fail("Docker copy container path is invalid")
      return { kind: "container" }
    })
    if (operands.filter((operand) => operand.kind === "host").length !== 1) {
      fail("Docker copy requires exactly one host path")
    }
    if (operands[0].kind !== "container" || operands[1].kind !== "host") {
      fail("raw Docker copy only supports container-to-host output")
    }
    validateSharedPath(operands.find((operand) => operand.kind === "host").path, "Docker copy host path")
    return
  }
  fail("Docker command shape is not allowed")
}

function validateProvisioner(action, environment, files) {
  if (!ACTIONS.has(action)) fail("provisioner action is not allowed")
  if (!environment || typeof environment !== "object" || Array.isArray(environment)) {
    fail("provisioner environment is invalid")
  }
  for (const [name, value] of Object.entries(environment)) {
    if (
      (!ALLOWED_ENVIRONMENT.has(name) && !/^CHARIOX_SLICE_DEVELOPMENT_MOUNT_[0-9]+$/.test(name)) ||
      typeof value !== "string" ||
      value.length > 512 * 1024 ||
      value.includes("\0")
    ) {
      fail(`provisioner environment field is not allowed: ${name}`)
    }
    if (PATH_ENVIRONMENT.has(name) || /^CHARIOX_SLICE_DEVELOPMENT_MOUNT_[0-9]+$/.test(name)) {
      validateSharedPath(value, name)
    }
  }
  const ownerPublicKey = environment.CHARIOX_SLICE_OWNER_PUBLIC_KEY
  if (ownerPublicKey !== undefined) {
    const decoded = /^[A-Za-z0-9+/]{87}=$/.test(ownerPublicKey)
      ? Buffer.from(ownerPublicKey, "base64")
      : null
    if (decoded?.length !== 65 || decoded[0] !== 4) {
      fail("relay owner public key is invalid")
    }
  }
  if (ROOM_BINDING_ENVIRONMENT.some(name => Object.hasOwn(environment, name))) {
    for (const name of ROOM_BINDING_ENVIRONMENT) {
      const value = environment[name]
      if (name === "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY") {
        const decoded = typeof value === "string" && /^[A-Za-z0-9+/]{87}=$/.test(value)
          ? Buffer.from(value, "base64") : null
        if (decoded?.length !== 65 || decoded[0] !== 4) fail("Room home public key is invalid")
      } else if (typeof value !== "string" || !/^[a-zA-Z0-9_.:-]{1,180}$/.test(value)) {
        fail(`${name} is invalid or missing`)
      }
    }
    if (environment.CHARIOX_ROOM_ENVIRONMENT_SLICE_ID !== environment.CHARIOX_SLICE_ID) {
      fail("Room binding does not match the provisioned slice")
    }
  }
  const commonEnvironment = new Set([
    "CHARIOX_SLICE_ID",
    "CHARIOX_SLICE_NAME",
    "CHARIOX_SLICE_HOME_VOLUME",
    "CHARIOX_SLICE_OWNER_KERNEL_ID",
    "CHARIOX_SLICE_OWNER_MACHINE_ID",
  ])
  const authEnvironment = new Set([
    ...commonEnvironment,
    "CHARIOX_SLICE_AUTH_PROVIDER",
    "CHARIOX_SLICE_ACCOUNT_OWNER",
    "CHARIOX_SLICE_ACCOUNT_PROFILE",
  ])
  const provisionsContainer = new Set(["provision", "restore-state"]).has(action)
  const usesFullEnvironment = provisionsContainer || action === "recover"
  const actionEnvironment = usesFullEnvironment
    ? ALLOWED_ENVIRONMENT
    : action === "start-provider-login"
      ? new Set([
        ...commonEnvironment,
        "CHARIOX_SLICE_LOGIN_PROVIDER",
        "CHARIOX_SLICE_ACCOUNT_OWNER",
        "CHARIOX_SLICE_ACCOUNT_PROFILE",
      ])
      : new Set(["import-provider-auth", "remove-provider-auth"]).has(action)
        ? authEnvironment
        : commonEnvironment
  for (const name of Object.keys(environment)) {
    if (!actionEnvironment.has(name) && !(usesFullEnvironment && /^CHARIOX_SLICE_DEVELOPMENT_MOUNT_[0-9]+$/.test(name))) {
      fail(`${name} is not allowed for provisioner action ${action}`)
    }
  }
  if (environment.CHARIOX_SLICE_EXTENSION_DOCKERFILE) {
    fail("managed slices do not accept extension Dockerfiles")
  }
  validateResource(environment.CHARIOX_SLICE_NAME ?? "", "slice container")
  if (environment.CHARIOX_SLICE_HOSTNAME !== undefined
      && !/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/.test(environment.CHARIOX_SLICE_HOSTNAME)) {
    fail("CHARIOX_SLICE_HOSTNAME is invalid")
  }
  if (!/^[a-zA-Z0-9_.:-]{1,180}$/.test(environment.CHARIOX_SLICE_ID ?? "")) {
    fail("CHARIOX_SLICE_ID is invalid")
  }
  if (provisionsContainer) {
    for (const [name, value] of Object.entries(environment)) {
      if (name === "CHARIOX_SLICE_WORKSPACE" || /^CHARIOX_SLICE_DEVELOPMENT_MOUNT_[0-9]+$/.test(name)) {
        // Preflight must enforce the same ownership layout as persistent mount creation.
        validatePersistentSource(resolve(value), environment.CHARIOX_SLICE_ID, name, SHARE_ROOT_INPUT)
      }
    }
  }
  for (const name of ["CHARIOX_SLICE_OWNER_KERNEL_ID", "CHARIOX_SLICE_OWNER_MACHINE_ID"]) {
    if (environment[name] && !/^[a-zA-Z0-9_.:-]{1,180}$/.test(environment[name])) {
      fail(`${name} is invalid`)
    }
  }
  validateResource(environment.CHARIOX_SLICE_HOME_VOLUME ?? "", "slice home volume")
  for (const name of ["CHARIOX_SLICE_ACCOUNT_OWNER", "CHARIOX_SLICE_ACCOUNT_PROFILE"]) {
    if (environment[name] && !/^[A-Za-z0-9-]{1,128}$/.test(environment[name])) fail(`${name} is invalid`)
  }
  if (environment.CHARIOX_SLICE_AUTH_PROVIDER && !/^(?:all|codex|claude|github|opencode(?::[A-Za-z0-9_.-]+)?)$/.test(environment.CHARIOX_SLICE_AUTH_PROVIDER)) {
    fail("CHARIOX_SLICE_AUTH_PROVIDER is invalid")
  }
  if (environment.CHARIOX_SLICE_LOGIN_PROVIDER && !/^(?:codex|claude|github|opencode(?::[A-Za-z0-9_.-]+)?)$/.test(environment.CHARIOX_SLICE_LOGIN_PROVIDER)) {
    fail("CHARIOX_SLICE_LOGIN_PROVIDER is invalid")
  }
  for (const name of ["CHARIOX_SLICE_DOCKER_IMAGE", "CHARIOX_SLICE_BASE_IMAGE"]) {
    if (environment[name] && !isDockerImageReference(environment[name])) fail(`${name} image reference is invalid`)
  }
  if (environment.CHARIOX_SLICE_BUILD_IMAGE && !["auto", "always", "never"].includes(environment.CHARIOX_SLICE_BUILD_IMAGE)) {
    fail("CHARIOX_SLICE_BUILD_IMAGE is invalid")
  }
  if (environment.CHARIOX_SLICE_VIEWER_BACKEND !== undefined && !VIEWER_BACKENDS.has(environment.CHARIOX_SLICE_VIEWER_BACKEND)) {
    fail("CHARIOX_SLICE_VIEWER_BACKEND is invalid")
  }
  if (environment.CHARIOX_SLICE_WORKSPACE_MOUNT_MODE && !["ro", "rw"].includes(environment.CHARIOX_SLICE_WORKSPACE_MOUNT_MODE)) {
    fail("CHARIOX_SLICE_WORKSPACE_MOUNT_MODE is invalid")
  }
  if (environment.CHARIOX_SLICE_APPARMOR_PROFILE && !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/.test(environment.CHARIOX_SLICE_APPARMOR_PROFILE)) {
    fail("CHARIOX_SLICE_APPARMOR_PROFILE is invalid")
  }
  if (environment.CHARIOX_SLICE_DOCKER_MEMORY && !/^[1-9][0-9]{0,6}[mMgG]$/.test(environment.CHARIOX_SLICE_DOCKER_MEMORY)) {
    fail("CHARIOX_SLICE_DOCKER_MEMORY is invalid")
  }
  if (environment.CHARIOX_SLICE_DOCKER_CPUS !== undefined && !isPositiveDockerCpuLimit(environment.CHARIOX_SLICE_DOCKER_CPUS)) {
    fail("CHARIOX_SLICE_DOCKER_CPUS is invalid")
  }
  for (const [name, minimum, maximum, grammar] of [
    ["CHARIOX_SLICE_DOCKER_PIDS_LIMIT", 1, 2_147_483_647, /^[1-9][0-9]{0,9}$/],
    ["CHARIOX_SLICE_DOCKER_NOFILE_LIMIT", 1024, 1_048_576, /^[1-9][0-9]{0,6}$/],
    ["CHARIOX_SLICE_MIN_FREE_MB", 0, 4_294_967_295, /^[0-9]{1,10}$/],
  ]) {
    const value = environment[name]
    if (value !== undefined && (!grammar.test(value) || Number(value) < minimum || Number(value) > maximum)) fail(`${name} is invalid`)
  }
  const diskLayerMb = environment.CHARIOX_SLICE_DISK_LAYER_MB
  const diskHomeMb = environment.CHARIOX_SLICE_DISK_HOME_MB
  if ((diskLayerMb === undefined) !== (diskHomeMb === undefined)) {
    fail("writable-layer and persistent-home disk limits must be configured together")
  }
  for (const [name, value] of [["CHARIOX_SLICE_DISK_LAYER_MB", diskLayerMb], ["CHARIOX_SLICE_DISK_HOME_MB", diskHomeMb]]) {
    if (value !== undefined && (!/^[1-9][0-9]{0,9}$/.test(value) || Number(value) > 4_294_967_295)) {
      fail(`${name} must be a positive u32 integer`)
    }
  }
  for (const name of [
    "CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP",
    "CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY",
    "CHARIOX_SLICE_START_DESKTOP",
    "CHARIOX_SLICE_START_PROVIDER_SERVERS",
    "CHARIOX_SLICE_START_RUNTIME",
    "CHARIOX_SLICE_IMPORT_PROVIDER_AUTH",
    "CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE",
  ]) {
    if (environment[name] && !["0", "1"].includes(environment[name])) fail(`${name} is invalid`)
  }
  for (const name of [
    "CHARIOX_SLICE_CODEX_PORT",
    "CHARIOX_SLICE_OPENCODE_PORT",
    "CHARIOX_SLICE_KERNEL_PORT",
    "CHARIOX_SLICE_MCP_PORT",
    "CHARIOX_SLICE_RELAY_PORT",
    "CHARIOX_SLICE_NOVNC_PORT",
  ]) {
    if (environment[name] && (!/^[0-9]{1,5}$/.test(environment[name]) || Number(environment[name]) > 65535)) {
      fail(`${name} is invalid`)
    }
  }
  for (const name of ["CHARIOX_SLICE_CODEX_PORT_RANGE", "CHARIOX_SLICE_OPENCODE_PORT_RANGE"]) {
    if (environment[name] && !/^[0-9]{1,5}-[0-9]{1,5}$/.test(environment[name])) fail(`${name} is invalid`)
  }
  const mountCount = environment.CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT ?? "0"
  if (!/^[0-9]{1,2}$/.test(mountCount) || Number(mountCount) > 32) {
    fail("CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT is invalid")
  }
  for (const name of Object.keys(environment).filter((name) => /^CHARIOX_SLICE_DEVELOPMENT_MOUNT_[0-9]+$/.test(name))) {
    const index = Number(name.slice("CHARIOX_SLICE_DEVELOPMENT_MOUNT_".length))
    if (index >= Number(mountCount)) fail(`${name} exceeds the declared mount count`)
  }
  if (!Array.isArray(files) || files.length > CREDENTIAL_ENVIRONMENT.size) {
    fail("provisioner credential files are invalid")
  }
  if (action !== "import-provider-auth" && files.length !== 0) {
    fail("provisioner credential files are only accepted for provider auth import")
  }
  const seen = new Set()
  let total = 0
  for (const file of files) {
    exactKeys(file, ["environment", "name", "contentsBase64"], "provisioner credential file")
    if (
      !CREDENTIAL_ENVIRONMENT.has(file.environment) ||
      seen.has(file.environment) ||
      typeof file.name !== "string" ||
      !/^[a-z0-9-]+\.(?:json|txt)$/.test(file.name) ||
      typeof file.contentsBase64 !== "string"
    ) {
      fail("provisioner credential file is invalid")
    }
    const contents = Buffer.from(file.contentsBase64, "base64")
    if (contents.toString("base64") !== file.contentsBase64 || contents.length > MAX_CREDENTIAL_BYTES) {
      fail("provisioner credential contents are invalid")
    }
    total += contents.length
    if (total > MAX_CREDENTIAL_TOTAL_BYTES) fail("provisioner credentials exceed their transfer limit")
    seen.add(file.environment)
  }
}

function validateRequest(request) {
  if (request?.kind === "docker") {
    exactKeys(request, ["kind", "args"], "Docker broker request")
    validateDocker(request.args)
    return
  }
  if (request?.kind === "provisioner") {
    exactKeys(request, ["kind", "action", "environment", "files"], "provisioner broker request")
    validateProvisioner(request.action, request.environment, request.files)
    return
  }
  if (request?.kind === "provider_auth_layout") {
    exactKeys(request, ["kind", "container"], "provider auth layout request")
    validateSliceContainer(request.container, "provider auth layout container")
    return
  }
  if (request?.kind === "capture_preflight") {
    exactKeys(request, ["kind", "container"], "slice capture preflight request")
    validateResource(request.container, "slice capture container")
    return
  }
  if (request?.kind === "home_restore_resolve") {
    exactKeys(request, ["kind", "container", "path"], "home restore resolution")
    validateSliceContainer(request.container, "home restore container")
    managedHomeArchiveCoordinates(request.path)
    return
  }
  if (request?.kind === "home_archive_capture") {
    exactKeys(request, ["kind", "container", "scope", "id"], "home archive capture request")
    validateResource(request.container, "home archive container")
    if (!/-home-archive-[0-9]+$/.test(request.container)) fail("home archive capture requires an archive helper")
    validateArtifactIdentity(request.scope, request.id)
    return
  }
  if (request?.kind === "home_archive_remove") {
    exactKeys(
      request,
      request.path === undefined ? ["kind", "scope", "id"] : ["kind", "scope", "id", "path"],
      "home archive remove request",
    )
    validateArtifactIdentity(request.scope, request.id)
    if (request.path !== undefined && (typeof request.path !== "string" || request.path.length > 4096)) {
      fail("home archive removal path is invalid")
    }
    return
  }
  if (request?.kind === "home_archive_verify") {
    exactKeys(request, ["kind", "scope", "id", "path"], "home archive verify request")
    validateArtifactIdentity(request.scope, request.id)
    if (typeof request.path !== "string" || request.path.length > 4096) {
      fail("home archive verification path is invalid")
    }
    const { relative } = managedHomeArchiveCoordinates(request.path)
    const expectedScope = request.scope === "state" ? "states" : "backups"
    if (relative[0] !== expectedScope || relative[1] !== request.id) {
      fail("home archive verification path does not match its identity")
    }
    return
  }
  fail("broker request kind is not allowed")
}

function validateArtifactIdentity(scope, id) {
  if (!new Set(["state", "backup"]).has(scope) || typeof id !== "string"
      || id === "." || id === ".." || !/^[a-zA-Z0-9_.:-]{1,180}$/.test(id)) {
    fail("home archive identity is invalid")
  }
}

function fsyncDirectory(path) {
  const fd = openSync(path, constants.O_RDONLY | constants.O_DIRECTORY)
  try {
    fsyncSync(fd)
  } finally {
    closeSync(fd)
  }
}

function artifactScopeRoot(scope) {
  return join(BROKER_ARTIFACT_ROOT, scope === "state" ? "states" : "backups")
}

function artifactDirectory(scope, id) {
  return join(artifactScopeRoot(scope), id)
}

async function captureHomeArchive(request) {
  // Capture only a verified protected or explicitly retained release F legacy
  // layout, with its exact home quiesced.
  const owner = request.container.match(/^(chariox-slice-[A-Za-z0-9_.:-]+)-home-archive-[0-9]+$/)?.[1]
  if (!owner) fail("slice home capture helper ownership is invalid")
  const layout = protectedLayouts.captureLayout(owner)
  protectedLayouts.requireQuiescedHome(owner)
  const scopeRoot = artifactScopeRoot(request.scope)
  mkdirSync(scopeRoot, { recursive: true, mode: 0o700 })
  chmodSync(scopeRoot, 0o700)
  const identityRoot = artifactDirectory(request.scope, request.id)
  if (request.scope === "backup" && existsSync(identityRoot)) fail("slice backup archive already exists")
  mkdirSync(identityRoot, { recursive: true, mode: 0o700 })
  chmodSync(identityRoot, 0o700)
  const staging = mkdtempSync(join(identityRoot, ".capture-"))
  chmodSync(staging, 0o700)
  const staged = join(staging, "home.tar.zst")
  try {
    // The verified helper streams its read-only home straight into private
    // broker state; the capture module bounds size, free space and duration.
    // Lease loss settles its process group like any other broker producer.
    const captured = await spawnBounded(process.execPath,
      [join(dirname(fileURLToPath(import.meta.url)), "protected-home-capture.mjs"), request.container, layout.homeVolume, staged,
        ...(layout.layoutKind === "legacy-release-f" ? ["legacy-release-f"] : [])],
      {env: dockerEnvironment(), maxBuffer: 64 * 1024,
        ...(layout.layoutKind === "legacy-release-f" ? {} : {timeout: 11 * 60_000})})
    if (captured.status !== 0) fail("slice home capture was refused; existing saved state is preserved")
    const result = JSON.parse(captured.stdout.toString("utf8"))
    exactKeys(result, ["sizeBytes", "sha256"], "protected home capture result")
    const expectedSize = result.sizeBytes
    const digest = result.sha256
    const metadata = lstatSync(staged)
    if (!Number.isSafeInteger(expectedSize) || expectedSize <= 0
        || !/^[a-f0-9]{64}$/.test(digest) || !metadata.isFile() || metadata.isSymbolicLink()
        || metadata.nlink !== 1 || metadata.size !== expectedSize || metadata.uid !== process.getuid()
        || (metadata.mode & 0o077) !== 0) fail("captured slice home archive is invalid")
    const stagedMetadata = join(staging, "metadata.json")
    const metadataFd = openSync(
      stagedMetadata,
      constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY,
      0o600,
    )
    try {
      writeFileSync(metadataFd, JSON.stringify({
        schemaVersion: 1,
        scope: request.scope,
        id: request.id,
        sizeBytes: expectedSize,
        sha256: digest,
      }))
      fsyncSync(metadataFd)
    } finally {
      closeSync(metadataFd)
    }
    fsyncDirectory(staging)
    const generation = `generation-${basename(staging).slice(".capture-".length)}`
    const destination = join(identityRoot, generation)
    renameSync(staging, destination)
    fsyncDirectory(identityRoot)
    fsyncDirectory(scopeRoot)
    const finalPath = join(destination, "home.tar.zst")
    protectedLayouts.recordCapture(owner, digest)
    return { path: finalPath, sizeBytes: expectedSize, sha256: digest }
  } finally {
    rmSync(staging, { recursive: true, force: true })
    if (request.scope === "backup" && existsSync(identityRoot) && readdirSync(identityRoot).length === 0) {
      rmSync(identityRoot, { recursive: true })
      fsyncDirectory(scopeRoot)
    }
  }
}

function managedHomeArchiveCoordinates(path) {
  const candidate = resolve(path)
  const relative = candidate.slice(BROKER_ARTIFACT_ROOT.length).split(sep).filter(Boolean)
  if (
    !candidate.startsWith(`${BROKER_ARTIFACT_ROOT}${sep}`) ||
    relative.length !== 4 ||
    !new Set(["states", "backups"]).has(relative[0]) ||
    !/^[a-zA-Z0-9_.:-]{1,180}$/.test(relative[1]) ||
    !/^generation-[a-zA-Z0-9]{6,64}$/.test(relative[2]) ||
    relative[3] !== "home.tar.zst"
  ) {
    fail("managed saved home archive path is invalid")
  }
  return { candidate, relative }
}

async function inspectManagedHomeArchive(path) {
  const { candidate, relative } = managedHomeArchiveCoordinates(path)
  const archive = pinnedSharedPath(candidate, "managed saved home archive", "file")
  let directory
  try {
    directory = pinnedSharedPath(dirname(candidate), "managed saved home archive generation", "directory")
    const entries = readdirSync(directory.path).sort()
    if (entries.length !== 2 || entries[0] !== "home.tar.zst" || entries[1] !== "metadata.json") {
      fail("managed saved home archive generation contains unexpected files")
    }
    const metadataPath = join(dirname(candidate), "metadata.json")
    const metadataFile = pinnedSharedPath(metadataPath, "managed saved home archive metadata", "file")
    try {
      const metadata = JSON.parse(readFileSync(metadataFile.path, "utf8"))
      exactKeys(metadata, ["schemaVersion", "scope", "id", "sizeBytes", "sha256"], "managed saved home archive metadata")
      const expectedScope = relative[0] === "states" ? "state" : "backup"
      const archiveMetadata = fstatSync(archive.fd)
      if (!homeArchiveMetadataMatches(metadata, expectedScope, relative[1], archiveMetadata.size)) {
        fail("managed saved home archive metadata is invalid")
      }
      const digest = await digestPinnedHomeArchive(archive.fd, HOME_ARCHIVE_PROGRESS_TIMEOUT_MS, brokerLifetime.signal)
      if (digest !== metadata.sha256) {
        fail("managed saved home archive digest does not match")
      }
      return { archive, directory, metadata }
    } finally {
      closeSync(metadataFile.fd)
    }
  } catch (error) {
    closeSync(archive.fd)
    if (directory) closeSync(directory.fd)
    throw error
  }
}

function verifyManagedHomeArchive(path) {
  return inspectManagedHomeArchive(path)
}

async function verifyHomeArchive(request) {
  const { archive, directory, metadata } = await inspectManagedHomeArchive(request.path)
  closeSync(archive.fd)
  closeSync(directory.fd)
  return {
    path: request.path,
    sizeBytes: metadata.sizeBytes,
    sha256: metadata.sha256,
  }
}

function removeHomeArchive(request) {
  const destination = artifactDirectory(request.scope, request.id)
  if (!existsSync(destination)) return
  const metadata = lstatSync(destination)
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) fail("home archive path is obstructed")
  if (request.path !== undefined) {
    const { candidate, relative } = managedHomeArchiveCoordinates(request.path)
    const expectedScope = request.scope === "state" ? "states" : "backups"
    if (relative[0] !== expectedScope || relative[1] !== request.id) {
      fail("home archive removal path does not match its identity")
    }
    const archive = pinnedSharedPath(candidate, "managed home archive removal", "file")
    closeSync(archive.fd)
    const generation = dirname(candidate)
    const generationMetadata = lstatSync(generation)
    if (generationMetadata.isSymbolicLink() || !generationMetadata.isDirectory()) {
      fail("home archive generation is obstructed")
    }
    rmSync(generation, { recursive: true })
    fsyncDirectory(destination)
    if (readdirSync(destination).length === 0) rmSync(destination, { recursive: true })
  } else {
    rmSync(destination, { recursive: true })
  }
  fsyncDirectory(artifactScopeRoot(request.scope))
}

function pinnedSharedPath(path, label, expectedKind) {
  const candidate = resolve(path)
  if (
    path.includes("\0") ||
    (candidate !== SHARE_ROOT_INPUT && !candidate.startsWith(`${SHARE_ROOT_INPUT}${sep}`))
  ) {
    fail(`${label} must stay under the managed slice share`)
  }
  const components = candidate.slice(SHARE_ROOT_INPUT.length).split(sep).filter(Boolean)
  let fd = openSync(SHARE_ROOT, LINUX_O_PATH | constants.O_NOFOLLOW | constants.O_DIRECTORY)
  try {
    for (const [index, component] of components.entries()) {
      if (component === "." || component === ".." || component.includes("\0")) fail(`${label} is invalid`)
      const final = index === components.length - 1
      const flags = final && expectedKind === "file"
        ? constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK
        : LINUX_O_PATH | constants.O_NOFOLLOW | constants.O_DIRECTORY
      const next = openSync(`/proc/${process.pid}/fd/${fd}/${component}`, flags)
      closeSync(fd)
      fd = next
    }
    const metadata = fstatSync(fd)
    if (
      (expectedKind === "directory" && !metadata.isDirectory()) ||
      (expectedKind === "file" && !metadata.isFile())
    ) {
      fail(`${label} is not a ${expectedKind}`)
    }
    const procPath = `/proc/${process.pid}/fd/${fd}`
    if (!isUnderShare(realpathSync(procPath))) fail(`${label} resolves outside the managed slice share`)
    return { fd, path: procPath }
  } catch (error) {
    closeSync(fd)
    throw error
  }
}

function handleMetadata(path) {
  try {
    return lstatSync(path)
  } catch (error) {
    if (error?.code === "ENOENT") return undefined
    throw error
  }
}

function spawnControl(command, args, options = {}) {
  // Fixed broker control commands never carry archive bytes. A stalled daemon
  // or utility must settle before admission or stable-handle mutation continues.
  return spawnSync(command, args, {
    ...options,
    timeout: options.timeout ?? 20_000,
    killSignal: "SIGKILL",
  })
}

function handleIsMountpoint(path) {
  const result = spawnControl("/usr/bin/mountpoint", ["-q", "--", path], { stdio: "ignore" })
  if (result.error || result.signal) fail("failed to inspect persistent mount handle")
  if (result.status === 0) return true
  if (result.status === 32) return false
  fail("failed to inspect persistent mount handle")
}

function unmountHandle(path) {
  if (!handleIsMountpoint(path)) return
  const result = spawnControl("/usr/bin/umount", [path], { stdio: "ignore" })
  if (result.status !== 0) fail("failed to unmount persistent mount handle")
}

function removeHandlePath(path) {
  const metadata = handleMetadata(path)
  if (!metadata) return
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    rmSync(path, { force: true })
    return
  }
  unmountHandle(path)
  rmSync(path, { recursive: true, force: true })
}

function publishHandle(record, fd) {
  mkdirSync(HANDLE_ROOT, { recursive: true, mode: 0o700 })
  chmodSync(HANDLE_ROOT, 0o700)
  const path = join(HANDLE_ROOT, record.handle)
  let metadata = handleMetadata(path)
  if (metadata?.isSymbolicLink()) {
    rmSync(path, { force: true })
    metadata = undefined
  }
  if (metadata && !metadata.isDirectory()) fail("persistent mount handle is obstructed")
  if (!metadata) mkdirSync(path, { mode: 0o700 })
  const expected = fstatSync(fd, { bigint: true })
  if (handleIsMountpoint(path)) {
    const current = lstatSync(path, { bigint: true })
    if (current.dev === expected.dev && current.ino === expected.ino) {
      persistentHandleDescriptors.set(record.handle, fd)
      return path
    }
    unmountHandle(path)
  } else if (readdirSync(path).length !== 0) {
    fail("persistent mount handle directory is not empty")
  }
  const mounted = spawnControl("/usr/bin/mount", ["--bind", "/proc/self/fd/3", path], {
    stdio: ["ignore", "ignore", "ignore", fd],
  })
  if (mounted.status !== 0) fail("failed to publish persistent mount handle")
  const published = lstatSync(path, { bigint: true })
  if (published.dev !== expected.dev || published.ino !== expected.ino) {
    unmountHandle(path)
    fail("persistent mount handle identity changed during publication")
  }
  fsyncDirectory(HANDLE_ROOT)
  persistentHandleDescriptors.set(record.handle, fd)
  return path
}

function persistHandleState() {
  mkdirSync(dirname(HANDLE_STATE), { recursive: true, mode: 0o700 })
  const temporary = `${HANDLE_STATE}.new-${process.pid}`
  rmSync(temporary, { force: true })
  const payload = Buffer.from(JSON.stringify({ schemaVersion: 1, handles: persistentHandleRecords }))
  if (payload.length > 1024 * 1024) fail("persistent mount handle state is too large")
  const fd = openSync(temporary, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY, 0o600)
  try {
    writeFileSync(fd, payload)
    fsyncSync(fd)
  } finally {
    closeSync(fd)
  }
  renameSync(temporary, HANDLE_STATE)
  const directory = openSync(dirname(HANDLE_STATE), constants.O_RDONLY | constants.O_DIRECTORY)
  try {
    fsyncSync(directory)
  } finally {
    closeSync(directory)
  }
}

function expectedHandle(container, slot) {
  return createHash("sha256").update(`${container}\0${slot}`).digest("hex")
}

function validatePersistentSource(source, sliceId, label, shareRoot = SHARE_ROOT) {
  const publication = join(shareRoot, "slices", "development", sliceId, "development")
  if (dirname(source) !== publication || basename(source).startsWith(".")) {
    fail(`${label} must be a direct repository in the matching slice publication`)
  }
}

function validateHandleRecord(record) {
  exactKeys(
    record,
    ["container", "dev", "handle", "ino", "rw", "sliceId", "slot", "source", "targets"],
    "persistent mount handle",
  )
  validateResource(record.container, "persistent mount container")
  if (!/^[a-zA-Z0-9_.:-]{1,180}$/.test(record.sliceId)) fail("persistent mount slice is invalid")
  if (!/^(?:workspace|development:[0-9]{1,2})$/.test(record.slot)) fail("persistent mount slot is invalid")
  if (
    typeof record.rw !== "boolean" ||
    !Array.isArray(record.targets) ||
    record.targets.length === 0 ||
    record.targets.length > 2 ||
    record.targets.some((target) => typeof target !== "string" || !target.startsWith("/") || target.includes("\0")) ||
    new Set(record.targets).size !== record.targets.length
  ) {
    fail("persistent mount targets are invalid")
  }
  if (
    record.handle !== expectedHandle(record.container, record.slot) ||
    !/^[0-9]+$/.test(record.dev) ||
    !/^[0-9]+$/.test(record.ino)
  ) {
    fail("persistent mount handle identity is invalid")
  }
  validatePersistentSource(record.source, record.sliceId, "persistent mount source")
}

function loadPersistentHandles() {
  if (persistentHandleRecords) return
  persistentHandleRecords = []
  mkdirSync(HANDLE_ROOT, { recursive: true, mode: 0o700 })
  chmodSync(HANDLE_ROOT, 0o700)
  const staleEntries = new Set(readdirSync(HANDLE_ROOT))
  if (!existsSync(HANDLE_STATE)) {
    for (const entry of staleEntries) removeHandlePath(join(HANDLE_ROOT, entry))
    return
  }
  const metadata = lstatSync(HANDLE_STATE)
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > 1024 * 1024) {
    fail("persistent mount handle state is invalid")
  }
  const state = JSON.parse(readFileSync(HANDLE_STATE, "utf8"))
  exactKeys(state, ["schemaVersion", "handles"], "persistent mount handle state")
  if (state.schemaVersion !== 1 || !Array.isArray(state.handles) || state.handles.length > MAX_PERSISTENT_HANDLES) {
    fail("persistent mount handle state is invalid")
  }
  const seenHandles = new Set()
  const seenSlots = new Set()
  let cleaned = false
  for (const record of state.handles) {
    let pinned
    try {
      validateHandleRecord(record)
      const slotIdentity = `${record.container}\0${record.slot}`
      if (seenHandles.has(record.handle) || seenSlots.has(slotIdentity)) fail("persistent mount handle is duplicated")
      pinned = pinnedSharedPath(record.source, "persistent mount source", "directory")
      const metadata = fstatSync(pinned.fd, { bigint: true })
      if (metadata.dev.toString() !== record.dev || metadata.ino.toString() !== record.ino) {
        closeSync(pinned.fd)
        pinned = undefined
        cleaned = true
        continue
      }
    } catch {
      if (pinned) closeSync(pinned.fd)
      cleaned = true
      continue
    }
    try {
      publishHandle(record, pinned.fd)
    } catch (error) {
      closeSync(pinned.fd)
      throw error
    }
    persistentHandleRecords.push(record)
    seenHandles.add(record.handle)
    seenSlots.add(`${record.container}\0${record.slot}`)
    staleEntries.delete(record.handle)
  }
  for (const entry of staleEntries) removeHandlePath(join(HANDLE_ROOT, entry))
  if (cleaned) persistHandleState()
}

function persistentSharedDirectory(path, container, sliceId, slot, targets, rw, label) {
  loadPersistentHandles()
  const pinned = pinnedSharedPath(path, label, "directory")
  const metadata = fstatSync(pinned.fd, { bigint: true })
  const source = realpathSync(pinned.path)
  validatePersistentSource(source, sliceId, label)
  const handle = expectedHandle(container, slot)
  const existing = persistentHandleRecords.find((record) => record.handle === handle)
  if (existing) {
    const oldFd = persistentHandleDescriptors.get(handle)
    if (oldFd === undefined) {
      closeSync(pinned.fd)
      fail("persistent mount source identity is unavailable")
    }
    const unchanged = existing.source === source &&
      existing.dev === metadata.dev.toString() &&
      existing.ino === metadata.ino.toString() &&
      existing.rw === rw &&
      JSON.stringify(existing.targets) === JSON.stringify(targets)
    if (unchanged) {
      closeSync(pinned.fd)
      return { created: false, handle, path: join(HANDLE_ROOT, handle) }
    }
    const replacement = {
      container,
      dev: metadata.dev.toString(),
      handle,
      ino: metadata.ino.toString(),
      rw,
      sliceId,
      slot,
      source,
      targets,
    }
    const index = persistentHandleRecords.indexOf(existing)
    try {
      publishHandle(replacement, pinned.fd)
    } catch (error) {
      publishHandle(existing, oldFd)
      closeSync(pinned.fd)
      throw error
    }
    persistentHandleRecords[index] = replacement
    try {
      persistHandleState()
    } catch (error) {
      publishHandle(existing, oldFd)
      persistentHandleRecords[index] = existing
      closeSync(pinned.fd)
      throw error
    }
    closeSync(oldFd)
    return { created: false, handle, path: join(HANDLE_ROOT, handle) }
  }
  if (persistentHandleRecords.length >= MAX_PERSISTENT_HANDLES) fail("persistent mount handle limit reached")
  const record = {
    container,
    dev: metadata.dev.toString(),
    handle,
    ino: metadata.ino.toString(),
    rw,
    sliceId,
    slot,
    source,
    targets,
  }
  persistentHandleRecords.push(record)
  try {
    const handlePath = publishHandle(record, pinned.fd)
    persistHandleState()
    return { created: true, handle, path: handlePath }
  } catch (error) {
    persistentHandleRecords.pop()
    if (persistentHandleDescriptors.get(handle) === pinned.fd) persistentHandleDescriptors.delete(handle)
    removeHandlePath(join(HANDLE_ROOT, handle))
    closeSync(pinned.fd)
    throw error
  }
}

function removePersistentHandles(predicate) {
  loadPersistentHandles()
  const removed = persistentHandleRecords.filter(predicate)
  if (removed.length === 0) return
  persistentHandleRecords = persistentHandleRecords.filter((record) => !predicate(record))
  persistHandleState()
  for (const record of removed) {
    const fd = persistentHandleDescriptors.get(record.handle)
    removeHandlePath(join(HANDLE_ROOT, record.handle))
    if (fd !== undefined) closeSync(fd)
    persistentHandleDescriptors.delete(record.handle)
  }
}

function releasePersistentHandles(container) {
  removePersistentHandles((record) => record.container === container)
}

function dockerEnvironment() {
  return { HOME: "/var/lib/chariox-docker/home", PATH: "/usr/bin:/bin", DOCKER_HOST }
}

function expectedProvisionerMounts(environment) {
  const container = environment.CHARIOX_SLICE_NAME
  const mode = environment.CHARIOX_SLICE_WORKSPACE_MOUNT_MODE ?? "rw"
  const mounts = environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT
    ? [
      {destination: "/var/lib/chariox/slice-private", source: environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT, rw: true},
      {destination: "/home/slice/.local/share/pki/nssdb", source: join(environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT, "nssdb"), rw: true},
    ] : []
  if (environment.CHARIOX_SLICE_WORKSPACE) {
    const targets = ["/workspace"]
    const mountCount = Number(environment.CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT ?? "0")
    if (mountCount === 0 && environment.CHARIOX_SLICE_WORKSPACE !== "/workspace") {
      targets.push(environment.CHARIOX_SLICE_WORKSPACE)
    }
    for (const target of targets) {
      mounts.push({
        destination: target,
        rw: mode === "rw",
        source: join(HANDLE_ROOT, expectedHandle(container, "workspace")),
      })
    }
  }
  const mountCount = Number(environment.CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT ?? "0")
  for (let index = 0; index < mountCount; index += 1) {
    mounts.push({
      destination: environment[`CHARIOX_SLICE_DEVELOPMENT_MOUNT_${index}`],
      rw: mode === "rw",
      source: join(HANDLE_ROOT, expectedHandle(container, `development:${index}`)),
    })
  }
  return mounts
}

function recordedContainerMounts(container) {
  loadPersistentHandles()
  return [...protectedLayouts.privateMounts(container), ...persistentHandleRecords
    .filter((record) => record.container === container)
    .flatMap((record) => record.targets.map((destination) => ({
      destination,
      rw: record.rw,
      source: join(HANDLE_ROOT, record.handle),
    })))]
}

function normalizedMounts(mounts) {
  return mounts
    .map((mount) => `${mount.source}\0${mount.destination}\0${mount.rw ? "rw" : "ro"}`)
    .sort()
}

function inspectContainerMounts(container) {
  const result = spawnControl(
    "/usr/bin/docker",
    ["container", "inspect", "--format", "{{json .Mounts}}", container],
    { env: dockerEnvironment(), encoding: "utf8", maxBuffer: 1024 * 1024 },
  )
  if (result.error || result.signal) fail("Docker container mount inspection failed")
  if (result.status !== 0) {
    if (result.status === 1 && dockerObjectNotFound(result.stderr, "container", container)) return undefined
    fail("Docker container mount inspection failed")
  }
  const mounts = JSON.parse(result.stdout)
  if (!Array.isArray(mounts)) fail("Docker container mount inspection is invalid")
  return mounts
    .filter((mount) => mount?.Type === "bind")
    .map((mount) => ({ destination: mount.Destination, rw: mount.RW === true, source: mount.Source }))
}

function diskQuotaMarkerPresent(container) {
  const inspectedContainer = spawnControl(
    "/usr/bin/docker",
    ["container", "inspect", "--format", "{{json .Config.Labels}}", container],
    { env: dockerEnvironment(), encoding: "utf8", maxBuffer: 256 * 1024, timeout: 20_000 },
  )
  if (readSliceDiskQuotaMarkerInspection(inspectedContainer, "container", container)) return true
  const volume = `${container}-home`
  const inspectedVolume = spawnControl(
    "/usr/bin/docker",
    ["volume", "inspect", "--format", "{{json .Labels}}", volume],
    { env: dockerEnvironment(), encoding: "utf8", maxBuffer: 256 * 1024, timeout: 20_000 },
  )
  return readSliceDiskQuotaMarkerInspection(inspectedVolume, "volume", volume)
}

function dockerInspection(result, label) {
  if (result.error || result.signal || result.status !== 0) {
    fail(`${label} could not be inspected`)
  }
  try {
    const value = JSON.parse(result.stdout)
    if (!value || typeof value !== "object" || Array.isArray(value)) fail(`${label} inspection is malformed`)
    return value
  } catch {
    fail(`${label} inspection is malformed`)
  }
}

function unboundedQuotaObservation(container) {
  const inspectedContainer = dockerInspection(spawnControl(
    "/usr/bin/docker",
    ["container", "inspect", "--format", "{{json .}}", container],
    { env: dockerEnvironment(), encoding: "utf8", maxBuffer: 1024 * 1024, timeout: 20_000 },
  ), "managed slice container")
  const containerLabels = inspectedContainer.Config?.Labels
  if (!containerLabels || typeof containerLabels !== "object" || Array.isArray(containerLabels)) {
    fail("managed slice container labels are unreadable")
  }
  if (readSliceDiskQuotaMarkerInspection({ status: 0, stdout: JSON.stringify(containerLabels) }, "container", container)) {
    fail("managed slice container has a disk quota marker")
  }
  if (!Array.isArray(inspectedContainer.Mounts)) fail("managed slice container mounts are unreadable")
  const homeMounts = inspectedContainer.Mounts.filter(
    (mount) => mount?.Type === "volume" && mount.Destination === "/home/slice",
  )
  if (homeMounts.length !== 1 || homeMounts[0].RW !== true || typeof homeMounts[0].Name !== "string") {
    fail("managed slice home-volume mount identity is invalid")
  }
  const homeVolumeName = homeMounts[0].Name
  const inspectedVolume = dockerInspection(spawnControl(
    "/usr/bin/docker",
    ["volume", "inspect", "--format", "{{json .}}", homeVolumeName],
    { env: dockerEnvironment(), encoding: "utf8", maxBuffer: 256 * 1024, timeout: 20_000 },
  ), "managed slice home volume")
  const volumeLabels = inspectedVolume.Labels
  if (!volumeLabels || typeof volumeLabels !== "object" || Array.isArray(volumeLabels)) {
    fail("managed slice home-volume labels are unreadable")
  }
  if (readSliceDiskQuotaMarkerInspection({ status: 0, stdout: JSON.stringify(volumeLabels) }, "volume", homeVolumeName)) {
    fail("managed slice home volume has a disk quota marker")
  }
  const identity = validateSliceDiskQuotaIdentity({
    ownerKernelId: containerLabels["io.chariox.slice.owner-kernel-id"],
    ownerMachineId: containerLabels["io.chariox.slice.owner-machine-id"],
    sliceId: containerLabels["io.chariox.slice.id"],
    containerName: container,
    homeVolumeName,
  })
  if (
    inspectedVolume.Name !== homeVolumeName ||
    volumeLabels["io.chariox.slice.owner-kernel-id"] !== identity.ownerKernelId ||
    volumeLabels["io.chariox.slice.owner-machine-id"] !== identity.ownerMachineId ||
    volumeLabels["io.chariox.slice.id"] !== identity.sliceId
  ) {
    fail("managed slice container and home-volume ownership do not match")
  }
  if (typeof inspectedContainer.Id !== "string" || !/^[a-f0-9]{64}$/.test(inspectedContainer.Id)) {
    fail("managed slice container identity is invalid")
  }
  if (
    typeof inspectedVolume.Mountpoint !== "string" || !isAbsolute(inspectedVolume.Mountpoint) ||
    typeof inspectedVolume.Driver !== "string" || typeof inspectedVolume.Scope !== "string"
  ) {
    fail("managed slice home-volume identity is invalid")
  }
  const mountpointMetadata = lstatSync(inspectedVolume.Mountpoint, { bigint: true })
  if (mountpointMetadata.isSymbolicLink() || !mountpointMetadata.isDirectory()) {
    fail("managed slice home-volume mountpoint identity is invalid")
  }
  return {
    identity,
    binding: {
      containerId: inspectedContainer.Id,
      homeVolumeCreatedAt: typeof inspectedVolume.CreatedAt === "string" ? inspectedVolume.CreatedAt : "",
      homeVolumeDevice: mountpointMetadata.dev.toString(),
      homeVolumeDriver: inspectedVolume.Driver,
      homeVolumeInode: mountpointMetadata.ino.toString(),
      homeVolumeMountpoint: inspectedVolume.Mountpoint,
      homeVolumeName,
      homeVolumeScope: inspectedVolume.Scope,
    },
  }
}

async function resolveBrokerUnboundedQuotaProof(container, lock) {
  const observed = unboundedQuotaObservation(container)
  return sliceDiskQuotaCoordinator.resolveUnboundedProof(lock, observed.identity, observed.binding)
}

async function rememberBrokerUnboundedQuotaProof(identity, container, lock) {
  try {
    sliceDiskQuotaCoordinator.assertLockHeld(lock)
    const observed = unboundedQuotaObservation(container)
    if (JSON.stringify(observed.identity) !== JSON.stringify(identity)) return
    await sliceDiskQuotaCoordinator.captureUnboundedProof(
      lock,
      observed.identity,
      observed.binding,
    )
  } catch {
    // Online allocator evidence remains sufficient for this operation; no offline proof is cached.
  }
}

function requireExactContainerMounts(container, expected, stop) {
  const actual = inspectContainerMounts(container)
  if (!actual) return false
  if (JSON.stringify(normalizedMounts(actual)) !== JSON.stringify(normalizedMounts(expected))) {
    fail("existing managed slice bind mounts do not match the broker-owned stable handle set")
  }
  if (stop) {
    const stopped = spawnControl("/usr/bin/docker", ["stop", container], {
      timeout: 30_000,
      env: dockerEnvironment(),
      maxBuffer: MAX_OUTPUT_BYTES,
    })
    if (stopped.status !== 0) fail("failed to stop managed slice before mount handle update")
  }
  return true
}

function stagedSharedOutput(path, label) {
  validateSharedPath(path, label)
  const name = basename(path)
  if (!name || name === "." || name === ".." || name.includes(sep)) fail(`${label} has an invalid name`)
  if (existsSync(path)) fail(`${label} already exists`)
  const parent = pinnedSharedPath(dirname(path), `${label} parent`, "directory")
  const outputRoot = realpathSync(BROKER_OUTPUT_ROOT)
  if (!isUnderShare(outputRoot)) fail("broker output root escaped the managed share")
  const stagingDirectory = mkdtempSync(join(outputRoot, "request-"))
  chmodSync(stagingDirectory, 0o700)
  return {
    fd: parent.fd,
    name,
    originalPath: path,
    parentPath: parent.path,
    path: join(stagingDirectory, "payload"),
    stagingDirectory,
  }
}

function publishStagedOutput(output) {
  const metadata = lstatSync(output.path)
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.nlink !== 1) {
    fail("Docker copy did not produce a safe regular file")
  }
  chmodSync(output.path, 0o660)
  linkSync(output.path, `${output.parentPath}/${output.name}`)
  unlinkSync(output.path)
}

function prepareDocker(args) {
  const prepared = [...args]
  const descriptors = []
  let output
  if (prepared[0] === "create" && prepared.length === 20) {
    const suffix = /-(?:disk-admission-[a-f0-9]{16}|home-archive-[0-9]{1,20})$/.exec(prepared[2])
    if (suffix) {
      const container = prepared[2].slice(0, suffix.index)
      prepared[16] = `${protectedLayouts.homeVolume(container)}:/home-src:ro`
    }
  }
  if (prepared[0] === "cp") {
    const hostIndex = prepared[1].includes(":") ? 2 : 1
    const hostPath = prepared[hostIndex]
    if (hostIndex !== 2) fail("Docker copy host sources are not allowed through the raw broker")
    const pinned = stagedSharedOutput(hostPath, "Docker copy host destination")
    descriptors.push(pinned.fd)
    prepared[hostIndex] = pinned.path
    if (hostIndex === 2) output = pinned
  }
  return { args: prepared, descriptors, output }
}

async function prepareProvisioner(request) {
  const environment = { ...request.environment }
  if (LOCAL_AUTHORITY) {
    environment.CHARIOX_SLICE_LOCAL_DEV_OWNER_UID = String(LOCAL_AUTHORITY.enrollment.ownerUid)
    environment.CHARIOX_SLICE_LOCAL_DEV_HELPER_NAME = process.env.CHARIOX_SLICE_LOCAL_DEV_HELPER_NAME
    environment.CHARIOX_SLICE_LOCAL_DEV_SOCKET_IDENTITY = process.env.CHARIOX_SLICE_LOCAL_DEV_SOCKET_IDENTITY
    environment.CHARIOX_SLICE_OWNED_WORKSPACE = "1"
    environment.CHARIOX_SLICE_BUILD_IMAGE = "never"
    // Saved images also refresh their worker runtime from the enrolled base.
    environment.CHARIOX_SLICE_BASE_IMAGE = LOCAL_AUTHORITY.enrollment.workerImageId
    if (["provision", "restore-state"].includes(request.action) && !environment.CHARIOX_SLICE_SAVED_HOME_ARCHIVE) {
      const requested = environment.CHARIOX_SLICE_DOCKER_IMAGE
      if (requested && !["chariox-slice-linux:0.1.0", LOCAL_AUTHORITY.enrollment.workerImageId].includes(requested)) fail("Local DEV worker image must match its installed enrollment")
      environment.CHARIOX_SLICE_DOCKER_IMAGE = LOCAL_AUTHORITY.enrollment.workerImageId
    }
  } else if (!environment.CHARIOX_SLICE_WORKSPACE) {
    // Without a development publication the provisioner would bind the signed
    // build context as /workspace; a managed slice gets its own volume instead.
    environment.CHARIOX_SLICE_OWNED_WORKSPACE = "1"
  }
  const privateRoot = protectedLayouts.prepare(request.action, environment)
  if (privateRoot) {
    environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT = privateRoot
    environment.CHARIOX_SLICE_HOME_VOLUME = protectedLayouts.homeVolume(environment.CHARIOX_SLICE_NAME)
  }
  const descriptors = []
  const handles = new Set()
  const newHandles = new Set()
  let inputDirectory
  let restoreDigest
  try {
    const provisionsContainer = new Set(["provision", "restore-state"]).has(request.action)
    if (provisionsContainer) {
      requireExactContainerMounts(
        environment.CHARIOX_SLICE_NAME,
        expectedProvisionerMounts(environment),
        true,
      )
    } else {
      const recorded = recordedContainerMounts(environment.CHARIOX_SLICE_NAME)
      if (recorded.length > 0) {
        requireExactContainerMounts(environment.CHARIOX_SLICE_NAME, recorded, false)
      } else if (
        new Set(["import-provider-auth", "remove-provider-auth", "start-provider-login"]).has(request.action) &&
        inspectContainerMounts(environment.CHARIOX_SLICE_NAME)
      ) {
        fail("existing managed slice has no broker-owned stable mount record")
      }
    }
    for (const name of provisionsContainer ? PATH_ENVIRONMENT : []) {
      const value = environment[name]
      if (!value) continue
      if (name === "CHARIOX_SLICE_WORKSPACE") {
        const persistent = persistentSharedDirectory(
          value,
          environment.CHARIOX_SLICE_NAME,
          environment.CHARIOX_SLICE_ID,
          "workspace",
          [
            "/workspace",
            ...(Number(environment.CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT ?? "0") === 0 && value !== "/workspace"
              ? [value]
              : []),
          ],
          (environment.CHARIOX_SLICE_WORKSPACE_MOUNT_MODE ?? "rw") === "rw",
          name,
        )
        environment.CHARIOX_SLICE_WORKSPACE_SOURCE = persistent.path
        handles.add(persistent.handle)
        if (persistent.created) newHandles.add(persistent.handle)
      } else {
        const pinned = name === "CHARIOX_SLICE_SAVED_HOME_ARCHIVE"
          ? await verifyManagedHomeArchive(value)
          : pinnedSharedPath(value, name, "file")
        if (name === "CHARIOX_SLICE_SAVED_HOME_ARCHIVE") {
          descriptors.push(pinned.archive.fd, pinned.directory.fd)
          environment[name] = pinned.archive.path
          environment.CHARIOX_SLICE_BROKER_SAVED_HOME_ARCHIVE_DIR = pinned.directory.path
          restoreDigest = pinned.metadata.sha256
        } else {
          descriptors.push(pinned.fd)
          environment[name] = pinned.path
        }
      }
    }
    const mountCount = provisionsContainer
      ? Number(environment.CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT ?? "0")
      : 0
    for (let index = 0; index < mountCount; index += 1) {
      const name = `CHARIOX_SLICE_DEVELOPMENT_MOUNT_${index}`
      const value = environment[name]
      if (!value) fail(`${name} is missing`)
      const persistent = persistentSharedDirectory(
        value,
        environment.CHARIOX_SLICE_NAME,
        environment.CHARIOX_SLICE_ID,
        `development:${index}`,
        [value],
        (environment.CHARIOX_SLICE_WORKSPACE_MOUNT_MODE ?? "rw") === "rw",
        name,
      )
      environment[`${name}_SOURCE`] = persistent.path
      handles.add(persistent.handle)
      if (persistent.created) newHandles.add(persistent.handle)
    }
    if (restoreDigest) protectedLayouts.beginRestore(environment, restoreDigest, request.action)
    if (request.action === "import-provider-auth") {
      mkdirSync(BROKER_INPUT_ROOT, { recursive: true, mode: 0o700 })
      chmodSync(BROKER_INPUT_ROOT, 0o700)
      inputDirectory = mkdtempSync(join(BROKER_INPUT_ROOT, "request-"))
      chmodSync(inputDirectory, 0o700)
      for (const credentialEnvironment of CREDENTIAL_ENVIRONMENT) {
        environment[credentialEnvironment] = join(inputDirectory, `absent-${credentialEnvironment.toLowerCase()}`)
      }
      for (const file of request.files) {
        const path = join(inputDirectory, file.name)
        writeFileSync(path, Buffer.from(file.contentsBase64, "base64"), { flag: "wx", mode: 0o600 })
        environment[file.environment] = path
      }
    }
    return { environment, descriptors, handles, inputDirectory, newHandles }
  } catch (error) {
    for (const fd of descriptors) closeSync(fd)
    if (inputDirectory) rmSync(inputDirectory, { recursive: true, force: true })
    if (newHandles.size > 0) removePersistentHandles((record) => newHandles.has(record.handle))
    throw error
  }
}

function cleanupPrepared(prepared) {
  for (const fd of prepared.descriptors) closeSync(fd)
  if (prepared.inputDirectory) rmSync(prepared.inputDirectory, { recursive: true, force: true })
  if (prepared.output?.stagingDirectory) rmSync(prepared.output.stagingDirectory, { recursive: true, force: true })
}

const brokerLifetime = new AbortController()
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => { brokerLifetime.abort() })
}

function inspectDockerObject(kind, reference) {
  const result = spawnSync("/usr/bin/docker", [kind, "inspect", reference],
    {env: dockerEnvironment(), encoding: "utf8", timeout: 30_000, maxBuffer: 1024 * 1024})
  if (result.status !== 0) fail("managed slice capture provenance is unavailable")
  const records = JSON.parse(result.stdout)
  if (!Array.isArray(records) || records.length !== 1) fail("managed slice capture provenance is unavailable")
  return records[0]
}

function seedLocalDevWorkerProof() {
  if (!LOCAL_AUTHORITY) return
  const enrollment = LOCAL_AUTHORITY.enrollment
  const image = inspectDockerObject("image", enrollment.workerImageId)
  if (image.Id !== enrollment.workerImageId || image.Config?.User !== "slice"
      || image.Config?.Labels?.["io.chariox.runtime-source-revision"] !== enrollment.workerRuntimeRevision) fail("Local DEV worker image identity mismatch")
  const runtime = spawnSync("/usr/bin/docker", ["run", "--rm", "--read-only", "--network", "none",
    "--cap-drop", "ALL", "--memory", "64m", "--pids-limit", "16", "--user", "0:0",
    "--entrypoint", "/usr/bin/sha256sum", enrollment.workerImageId, "/opt/chariox-slice/bin/chariox-kernel"],
    {env: dockerEnvironment(), encoding: "utf8", maxBuffer: 1024, timeout: 30_000})
  if (runtime.status !== 0 || runtime.stdout.trim().split(/\s+/)[0] !== enrollment.workerKernelHash) fail("Local DEV runtime image hash mismatch")
  // Root-installed explicit DEV enrollment is this proof's authority. It makes
  // no managed signature claim. The same image/lineage checks protect capture.
  recordManagedImageProof(protectedLayouts.imageRoot, enrollment.sourceDigest, image, enrollment.workerKernelHash)
}

function spawnBounded(command, args, options) {
  return runBrokerCommand(command, args, { ...options, signal: brokerLifetime.signal, logRoot: join(BROKER_OUTPUT_ROOT, "logs") })
}

function provisionerQuotaRequest(environment) {
  const identity = sliceDiskQuotaIdentityFromEnvironment(environment)
  const layerMb = environment.CHARIOX_SLICE_DISK_LAYER_MB
  const homeMb = environment.CHARIOX_SLICE_DISK_HOME_MB
  if ((layerMb === undefined) !== (homeMb === undefined)) {
    fail("writable-layer and persistent-home disk limits must be configured together")
  }
  return {
    identity,
    limits: layerMb === undefined
      ? undefined
      : {
        writableLayerBytes: Number(layerMb) * 1024 * 1024,
        persistentHomeBytes: Number(homeMb) * 1024 * 1024,
      },
  }
}

async function execute(request) {
  validateRequest(request)
  // Release F's disk-quota allocator, its coordination root and start-admission
  // proofs exist only on managed hosts. The Phase 1 local DEV broker has none of
  // them, so its slices stay unbounded and bypass that coordination; it refuses
  // quota limits and never provisions or starts a quota-marked slice.
  const quotaCoordinated = !LOCAL_AUTHORITY
  if (LOCAL_AUTHORITY) {
    verifiedProtectedAuthority()
    if (request.kind === "provisioner" && (request.environment.CHARIOX_SLICE_WORKSPACE
        || Number(request.environment.CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT ?? "0") !== 0)) {
      fail("Local protected DEV slices do not support host workspace or development mounts")
    }
    if (request.kind === "provisioner" && (request.environment.CHARIOX_SLICE_DISK_LAYER_MB !== undefined
        || request.environment.CHARIOX_SLICE_DISK_HOME_MB !== undefined)) {
      fail("Local protected DEV slices do not support managed disk quotas")
    }
    const unadmittedSlice = request.kind === "docker" && ["start", "unpause"].includes(request.args[0])
      ? request.args[1]
      : request.kind === "provisioner" && ["provision", "restore-state", "recover"].includes(request.action)
        ? request.environment.CHARIOX_SLICE_NAME
        : undefined
    if (unadmittedSlice && diskQuotaMarkerPresent(unadmittedSlice)) {
      fail("a disk-quota slice cannot run without the managed quota allocator")
    }
  }
  if (request.kind === "docker" && request.args[0] === "exec"
      && PROTECTED_PROVIDER_ACCOUNT_CREDENTIAL_PATH.test(request.args.at(-1))) {
    protectedLayouts.preflight(request.args[3])
  }
  if (request.kind === "docker" && request.args[0] === "exec" && request.args[7] === "slice-runtime-logs") {
    const protectedLayout = protectedLayouts.providerAuthProtected(request.args[3])
    if (request.args[9] !== (protectedLayout ? "protected" : "legacy")) fail("slice runtime log layout does not match verified container")
  }
  let commitSource
  let commitParent
  let legacyCommit = false
  let flattenCapture = false
  if (request.kind === "docker" && request.args[0] === "commit") {
    const layout = protectedLayouts.captureLayout(request.args[1])
    legacyCommit = layout.layoutKind === "legacy-release-f"
    protectedLayouts.requireQuiescedHome(request.args[1])
    if (!legacyCommit) requireSafeHomeVolume({volume: layout.homeVolume,
      docker: args => spawnSync("/usr/bin/docker", args, {env: dockerEnvironment(), timeout: 30_000, maxBuffer: 8 * 1024 * 1024})})
    commitSource = inspectDockerObject("container", request.args[1])
    commitParent = inspectDockerObject("image", commitSource.Image)
    // A capture on a deep saved image flattens instead of adding a layer, so
    // saved images never reach Docker's layer limit (captured-image-depth.mjs).
    flattenCapture = captureNeedsFlatten(commitParent.RootFS?.Layers?.length)
  }
  if (request.kind === "provider_auth_layout") {
    const protectedLayout = protectedLayouts.providerAuthProtected(request.container)
    return {status: 0, stdoutBase64: Buffer.from(JSON.stringify({protectedLayout})).toString("base64"), stderrBase64: ""}
  }
  if (request.kind === "capture_preflight") {
    const layout = protectedLayouts.captureLayout(request.container)
    if (layout.layoutKind === "legacy-release-f") {
      return {status: 0, stdoutBase64: "", stderrBase64: Buffer.from(LEGACY_CAPTURE_NOTICE).toString("base64")}
    }
    requireSafeHomeVolume({volume: layout.homeVolume, quiesced: false,
      docker: args => spawnSync("/usr/bin/docker", args, {env: dockerEnvironment(), timeout: 30_000, maxBuffer: 8 * 1024 * 1024})})
    return {status: 0, stdoutBase64: "", stderrBase64: ""}
  }
  if (request.kind === "home_restore_resolve") {
    const {archive, directory, metadata} = await inspectManagedHomeArchive(request.path)
    try { protectedLayouts.resolveRestore(request.container, metadata.sha256) }
    finally { closeSync(archive.fd); closeSync(directory.fd) }
    return {status: 0, stdoutBase64: "", stderrBase64: ""}
  }
  let releaseDiskQuota = false
  let retainedHomes = []
  let unboundedQuotaIdentity
  let unboundedQuotaStatusVerified = false
  if (request.kind === "home_archive_capture") {
    const captured = await captureHomeArchive(request)
    return { status: 0, stdoutBase64: Buffer.from(JSON.stringify(captured)).toString("base64"), stderrBase64: "" }
  }
  if (request.kind === "home_archive_remove") {
    removeHomeArchive(request)
    return { status: 0, stdoutBase64: "", stderrBase64: "" }
  }
  if (request.kind === "home_archive_verify") {
    const verified = await verifyHomeArchive(request)
    return { status: 0, stdoutBase64: Buffer.from(JSON.stringify(verified)).toString("base64"), stderrBase64: "" }
  }
  if (request.kind === "docker" && request.args[0] === "start") {
    const recorded = recordedContainerMounts(request.args[1])
    if (recorded.length > 0) {
      requireExactContainerMounts(request.args[1], recorded, false)
    } else if (
      !/-home-archive-[0-9]+$/.test(request.args[1]) &&
      !isDiskAdmissionHelper(request.args[1])
    ) {
      fail("managed slice start has no broker-owned stable mount record")
    }
  }
  if (request.kind === "provisioner" && ["provision", "restore-state", "recover", "destroy"].includes(request.action)) {
    // Quota requests must use the broker's retained home, not a kernel default
    // that predates a protected restore generation.
    request.environment.CHARIOX_SLICE_HOME_VOLUME = protectedLayouts.homeVolume(request.environment.CHARIOX_SLICE_NAME)
  }
  let boundedLimits
  if (quotaCoordinated && request.kind === "provisioner" && ["provision", "restore-state", "recover"].includes(request.action)) {
    const quota = provisionerQuotaRequest(request.environment)
    unboundedQuotaIdentity = quota.identity
    if (quota.limits) {
      await requestSliceDiskQuota({
        protocolVersion: 1,
        operation: "reserve",
        identity: quota.identity,
        limits: quota.limits,
      })
      boundedLimits = quota.limits
    } else {
      if (diskQuotaMarkerPresent(quota.identity.containerName)) {
        fail("configured disk quota caps are missing for a slice with an existing reservation")
      }
      try {
        const status = await requestSliceDiskQuota({
          protocolVersion: 1,
          operation: "status",
          identity: quota.identity,
        })
        if (status.bounded) fail("configured disk quota caps are missing for a slice with an existing reservation")
        if (!status || typeof status !== "object" || Array.isArray(status) || status.bounded !== false) {
          fail("managed disk quota allocator status is invalid")
        }
        exactKeys(status, ["bounded"], "managed disk quota unbounded status")
        unboundedQuotaStatusVerified = true
      } catch (error) {
        if (!new Set(["ENOENT", "ECONNREFUSED"]).has(error?.code)) throw error
      }
    }
  }
  if (request.kind === "provisioner" && request.action === "destroy") {
    // MP-03/MP-08/MP-10/MP-11: retained generations belong to the slice
    // lifecycle, including unbounded DEV homes with no quota reservation.
    retainedHomes = protectedLayouts.retainedHomeVolumes(request.environment.CHARIOX_SLICE_NAME)
  }
  if (quotaCoordinated && request.kind === "provisioner" && request.action === "destroy") {
    const quota = provisionerQuotaRequest(request.environment)
    releaseDiskQuota = diskQuotaMarkerPresent(quota.identity.containerName)
    try {
      const status = await requestSliceDiskQuota({
        protocolVersion: 1,
        operation: "status",
        identity: quota.identity,
      })
      releaseDiskQuota ||= status.bounded
    } catch (error) {
      if (!new Set(["ENOENT", "ECONNREFUSED"]).has(error?.code)) releaseDiskQuota = true
    }
  }
  let prepared
  try {
    const runPrepared = async (admission) => {
      prepared = request.kind === "docker" ? prepareDocker(request.args) : await prepareProvisioner(request)
      if (request.kind === "docker" && admission?.source === "broker-proof") {
        prepared.args[1] = admission.containerId
      }
      if (flattenCapture) {
        return spawnBounded(process.execPath, [join(dirname(fileURLToPath(import.meta.url)), "captured-image-depth.mjs"),
          "flatten", commitSource.Id, prepared.args[2]], {env: dockerEnvironment(), maxBuffer: MAX_OUTPUT_BYTES})
      }
      const command = request.kind === "docker" ? "/usr/bin/docker" : PROVISIONER
      const args = request.kind === "docker" ? prepared.args : [request.action]
      const env = request.kind === "docker"
        ? dockerEnvironment()
        : {
          HOME: "/var/lib/chariox-docker/home",
          PATH: "/usr/local/bin:/usr/bin:/bin",
          DOCKER_HOST,
          ...prepared.environment,
          ...(LOCAL_AUTHORITY ? {CHARIOX_SLICE_LOCAL_DEV_OWNER_UID: String(LOCAL_AUTHORITY.enrollment.ownerUid),
            CHARIOX_SLICE_LOCAL_DEV_HELPER_NAME: process.env.CHARIOX_SLICE_LOCAL_DEV_HELPER_NAME,
            CHARIOX_SLICE_BUILD_IMAGE: "never",
            ...localDevRuntimeEnvironment(LOCAL_AUTHORITY.enrollment)} : {}),
          ...(VERIFIED_BUILD_CONTEXT_DIGEST
            ? { CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: VERIFIED_BUILD_CONTEXT_DIGEST,
              ...(protectedLayouts.trusted ? {CHARIOX_SLICE_PROTECTED_IMAGE_PROOF_ROOT: protectedLayouts.imageRoot} : {}) }
            : {}),
        }
      return spawnBounded(command, args, {
        env, maxBuffer: MAX_OUTPUT_BYTES,
        ...(request.kind === "docker" ? dockerControlPolicy(request.args) : {}),
      })
    }
    const containerName = request.kind === "docker" ? request.args[1] : undefined
    const isDockerStartOrUnpause = request.kind === "docker" && ["start", "unpause"].includes(request.args[0]) && quotaCoordinated
    const isQuotaProvision = request.kind === "provisioner" && ["provision", "restore-state", "recover"].includes(request.action) && quotaCoordinated
    const result = isDockerStartOrUnpause
      ? await sliceDiskQuotaCoordinator.withContainerLock(containerName, async (lock) => runWithSliceDiskQuotaAdmission({
        containerName,
        quotaMarkerPresent: diskQuotaMarkerPresent(containerName),
        resolveUnboundedProof: () => resolveBrokerUnboundedQuotaProof(containerName, lock),
        run: async (quotaResult, admission) => {
          sliceDiskQuotaCoordinator.assertLockHeld(lock)
          let durableUnboundedState
          if (admission.source === "allocator") {
            if (quotaResult.bounded) {
              await sliceDiskQuotaCoordinator.assertBounded(lock)
            } else {
              durableUnboundedState = await sliceDiskQuotaCoordinator.assertUnbounded(lock, { allowMissing: true })
            }
          } else {
            const current = unboundedQuotaObservation(containerName)
            const currentProof = await sliceDiskQuotaCoordinator.resolveUnboundedProof(
              lock,
              current.identity,
              current.binding,
            )
            if (currentProof?.containerId !== admission.containerId) {
              fail("broker-owned unbounded quota proof changed before container start")
            }
          }

          let before
          if (admission.source === "allocator" && quotaResult.bounded === false) {
            try {
              before = unboundedQuotaObservation(containerName)
            } catch {
              // Online allocator evidence remains sufficient; without a stable binding no receipt is cached.
            }
          }
          sliceDiskQuotaCoordinator.assertLockHeld(lock)
          const started = await runPrepared(admission)
          if (before && durableUnboundedState && started.status === 0) {
            try {
              const after = unboundedQuotaObservation(containerName)
              if (
                JSON.stringify(before.identity) === JSON.stringify(after.identity) &&
                JSON.stringify(before.binding) === JSON.stringify(after.binding)
              ) {
                await sliceDiskQuotaCoordinator.captureUnboundedProof(
                  lock,
                  after.identity,
                  after.binding,
                )
              }
            } catch {
              // Keep the successful start result; a failed proof write means future outages fail closed.
            }
          }
          return started
        },
      }))
      : isQuotaProvision
        ? await sliceDiskQuotaCoordinator.withContainerLock(
          unboundedQuotaIdentity.containerName,
          async (lock) => {
            let durableUnboundedState
            if (boundedLimits) {
              await sliceDiskQuotaCoordinator.assertBounded(lock, {
                identity: unboundedQuotaIdentity,
                limits: boundedLimits,
              })
            } else if (unboundedQuotaStatusVerified) {
              durableUnboundedState = await sliceDiskQuotaCoordinator.assertUnbounded(lock, { allowMissing: true })
            } else {
              await sliceDiskQuotaCoordinator.assertUnbounded(lock)
            }
            const provisioned = await runPrepared()
            if (provisioned.status === 0 && unboundedQuotaStatusVerified && durableUnboundedState) {
              await rememberBrokerUnboundedQuotaProof(
                sliceDiskQuotaIdentityFromEnvironment(prepared.environment),
                request.environment.CHARIOX_SLICE_NAME,
                lock,
              )
            }
            return provisioned
          },
        )
      : await runPrepared()
    if (commitSource && result.status === 0) {
      const captured = inspectDockerObject("image", request.args[2])
      // A flattened capture is bound to the image ID its helper imported, not to the tag.
      const imported = flattenCapture ? flattenedImageId(result.stdout) : undefined
      if (flattenCapture && captured.Id !== imported) fail("flattened slice capture changed before its proof")
      if (legacyCommit) protectedLayouts.recordLegacyImage(request.args[1], commitParent, commitSource, captured,
        {flattened: flattenCapture})
      else if (flattenCapture) recordFlattenedImageProof(protectedLayouts.imageRoot, VERIFIED_BUILD_CONTEXT_DIGEST,
        commitParent, commitSource, captured, imported)
      else recordCapturedImageProof(protectedLayouts.imageRoot, VERIFIED_BUILD_CONTEXT_DIGEST,
        commitParent, commitSource, captured)
    }
    if (request.kind === "docker" && prepared.output && result.status === 0) {
      publishStagedOutput(prepared.output)
    }
    if (request.kind === "provisioner" && request.action === "destroy" && result.status === 0) {
      if (quotaCoordinated) {
        const identity = sliceDiskQuotaIdentityFromEnvironment(request.environment)
        await sliceDiskQuotaCoordinator.withContainerLock(identity.containerName, (lock) => {
          sliceDiskQuotaCoordinator.revokeUnboundedProof(lock, identity)
        })
      }
      releasePersistentHandles(request.environment.CHARIOX_SLICE_NAME)
      // MP-03/MP-08/MP-10/MP-11: an empty unbounded cleanup needs no quota
      // identity or allocator. Recorded homes still retire before release.
      if (retainedHomes.length > 0 || releaseDiskQuota) {
        const quota = provisionerQuotaRequest(request.environment)
        retireProtectedQuotaHomes(retainedHomes, quota.identity,
          args => spawnControl("/usr/bin/docker", args, {env: dockerEnvironment(), timeout: 30_000, maxBuffer: 1024 * 1024}))
        if (releaseDiskQuota) {
          await requestSliceDiskQuota({
            protocolVersion: 1,
            operation: "release",
            identity: quota.identity,
          })
        }
      }
    }
    if (request.kind === "provisioner" && new Set(["provision", "restore-state"]).has(request.action)) {
      if (result.status === 0) {
        protectedLayouts.complete(prepared.environment)
        removePersistentHandles(
          (record) => record.container === request.environment.CHARIOX_SLICE_NAME && !prepared.handles.has(record.handle),
        )
      } else {
        removePersistentHandles((record) => prepared.newHandles.has(record.handle))
      }
    }
    const response = {
      status: result.status ?? 125,
      stdoutBase64: (result.stdout ?? Buffer.alloc(0)).toString("base64"),
      stderrBase64: (result.stderr ?? Buffer.from(result.error?.message ?? "")).toString("base64"),
    }
    if (result.status === 0 && boundedLimits) {
      const quota = provisionerQuotaRequest(prepared.environment)
      const verified = await requestSliceDiskQuota({
        protocolVersion: 1,
        operation: "verify",
        identity: quota.identity,
      })
      response.diskQuotaEvidence = verified.evidence
    }
    return response
  } finally {
    if (prepared) cleanupPrepared(prepared)
  }
}

function errorResponse(error) {
  return {
    status: 125,
    stdoutBase64: "",
    stderrBase64: Buffer.from(error instanceof Error ? error.message : String(error)).toString("base64"),
  }
}

function responsePayload(response) {
  const payload = Buffer.from(JSON.stringify(response))
  if (payload.length <= MAX_FRAME_BYTES) return payload
  return Buffer.from(JSON.stringify(errorResponse(new Error("broker response is too large"))))
}

function serializeResponse(response) {
  const payload = responsePayload(response)
  const frame = Buffer.allocUnsafe(4 + payload.length)
  frame.writeUInt32BE(payload.length)
  payload.copy(frame, 4)
  return frame
}

if (process.argv[2] === "--validate-request") {
  const chunks = []
  for await (const chunk of process.stdin) chunks.push(chunk)
  try {
    validateRequest(JSON.parse(Buffer.concat(chunks)))
    process.stdout.write("ok\n")
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
} else if (process.argv[2] === "--stdio") {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity })
  for await (const line of lines) {
    let response
    try {
      if (Buffer.byteLength(line) > MAX_FRAME_BYTES) fail("broker request is too large")
      response = await execute(JSON.parse(line))
    } catch (error) {
      response = errorResponse(error)
    }
    process.stdout.write(`${responsePayload(response).toString()}\n`)
  }
} else {
  seedLocalDevWorkerProof()
  rmSync(BROKER_INPUT_ROOT, { recursive: true, force: true })
  mkdirSync(BROKER_INPUT_ROOT, { recursive: true, mode: 0o700 })
  const outputMetadata = lstatSync(BROKER_OUTPUT_ROOT)
  if (outputMetadata.isSymbolicLink() || !outputMetadata.isDirectory()) fail("broker output root is invalid")
  chmodSync(BROKER_OUTPUT_ROOT, 0o700)
  for (const entry of readdirSync(BROKER_OUTPUT_ROOT)) {
    rmSync(join(BROKER_OUTPUT_ROOT, entry), { recursive: true, force: true })
  }
  const artifactMetadata = lstatSync(BROKER_ARTIFACT_ROOT)
  if (artifactMetadata.isSymbolicLink() || !artifactMetadata.isDirectory()) fail("broker artifact root is invalid")
  chmodSync(BROKER_ARTIFACT_ROOT, 0o700)
  for (const entry of readdirSync(BROKER_ARTIFACT_ROOT)) {
    if (entry.startsWith(".capture-")) rmSync(join(BROKER_ARTIFACT_ROOT, entry), { recursive: true, force: true })
  }
  loadPersistentHandles()
  rmSync(SOCKET_PATH, { force: true })
  let accepted = false
  const server = createServer((socket) => {
    if (accepted) {
      socket.destroy()
      return
    }
    accepted = true
    server.close()
    rmSync(SOCKET_PATH, { force: true })
    socket.on("close", () => brokerLifetime.abort())
    socket.on("error", () => brokerLifetime.abort())
    let buffered = Buffer.alloc(0)
    let processing = Promise.resolve()
    socket.on("data", (chunk) => {
      buffered = Buffer.concat([buffered, chunk])
      while (buffered.length >= 4) {
        const payloadLength = buffered.readUInt32BE(0)
        if (payloadLength === 0 || payloadLength > MAX_FRAME_BYTES) {
          socket.destroy(new Error("broker request frame is invalid"))
          return
        }
        if (buffered.length < 4 + payloadLength) return
        const payload = Buffer.from(buffered.subarray(4, 4 + payloadLength))
        const remainder = Buffer.from(buffered.subarray(4 + payloadLength))
        buffered.fill(0)
        buffered = remainder
        let parsed
        try {
          parsed = JSON.parse(payload.toString("utf8"))
        } catch (error) {
          payload.fill(0)
          socket.write(serializeResponse(errorResponse(error)))
          continue
        }
        payload.fill(0)
        processing = processing.then(async () => {
          let response
          try {
            if (brokerLifetime.signal.aborted) throw new Error("slice broker lease closed")
            response = await execute(parsed)
          } catch (error) {
            response = errorResponse(error)
          }
          socket.write(serializeResponse(response))
        })
      }
    })
  })
  server.listen(SOCKET_PATH, () => {
    if (LOCAL_AUTHORITY) {
      chownSync(SOCKET_PATH, LOCAL_AUTHORITY.enrollment.ownerUid, LOCAL_AUTHORITY.enrollment.ownerGid)
      chmodSync(SOCKET_PATH, 0o600)
    } else chmodSync(SOCKET_PATH, 0o660)
  })
  setTimeout(() => {
    if (!accepted) {
      server.close()
      rmSync(SOCKET_PATH, { force: true })
    }
  }, 6000).unref()
}

function requireSafeHomeVolume(options) {
  if (LOCAL_AUTHORITY) return requireSafeLocalHomeVolume({...options, enrollment: LOCAL_AUTHORITY.enrollment})
  return requireSafeManagedHomeVolume(options)
}
