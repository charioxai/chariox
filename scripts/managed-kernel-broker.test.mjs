import "./managed-provisioner-progress.test.mjs"
import "./managed-home-archive-digest.test.mjs"
import assert from "node:assert/strict"
import { access, chmod, mkdtemp, mkdir, readFile, readdir, readlink, rename, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"
import { spawn, spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { once } from "node:events"
import { createConnection } from "node:net"
import { test } from "node:test"
import { runInNewContext } from "node:vm"
import { dockerObjectNotFound } from "../apps/kernel/slice-linux-docker/slice-disk-quota-admission.mjs"
import { missingBrokerDockerFixture } from "./lib/broker-docker-fixture.mjs"
import * as archivePolicy from "../apps/kernel/slice-linux-docker/managed-home-archive-stream.mjs"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const broker = join(
  repositoryRoot,
  "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
)
// The production broker is an ES module; these isolated VM fixtures are scripts.
// Preserve its module URL when evaluating the complete execution path, including
// the saved-image flatten helper added in #822.
function brokerExecutionFixture(source, context) {
  const spawnStart = source.indexOf("function spawnBounded(")
  const spawnEnd = source.indexOf("\nfunction provisionerQuotaRequest(", spawnStart)
  const executeStart = source.indexOf("async function execute(request)")
  const executeEnd = source.indexOf("\nfunction errorResponse(", executeStart)
  assert.ok(spawnStart >= 0 && spawnEnd > spawnStart, "broker owned-lifetime function must exist")
  assert.ok(executeStart >= 0 && executeEnd > executeStart, "broker execution function must exist")
  const spawnSource = source.slice(spawnStart, spawnEnd)
  const executeSource = source.slice(executeStart, executeEnd)
    .replaceAll("import.meta.url", JSON.stringify(pathToFileURL(broker).href))
  return runInNewContext(`${spawnSource}\n${executeSource}\nexecute`, { dirname, fileURLToPath, ...context })
}

const ownerPublicKey = Buffer.concat([Buffer.from([4]), Buffer.alloc(64, 1)]).toString("base64")
const sliceRuntimeLogScript = `
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

function validate(request, shareRoot) {
  return spawnSync(process.execPath, [broker, "--validate-request"], {
    input: JSON.stringify(request),
    encoding: "utf8",
    env: {
      ...process.env,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: shareRoot,
      CHARIOX_SLICE_DOCKER_BROKER_ARTIFACT_ROOT: join(shareRoot, ".broker-private/artifacts"),
    },
  })
}

test("canonical slice worker IDs cross managed provision and recovery without widening other actions", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-worker-identity-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = JSON.parse(await readFile(join(repositoryRoot, "fixtures/slice-worker-identity.json"), "utf8")).cases[0]
  const environment = {
    CHARIOX_SLICE_NAME: "chariox-slice-dev", CHARIOX_SLICE_ID: "slice-dev",
    CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
    CHARIOX_SLICE_OWNER_KERNEL_ID: fixture.ownerKernelId,
    CHARIOX_SLICE_OWNER_MACHINE_ID: fixture.machineId,
    CHARIOX_SLICE_DAEMON_ID: fixture.workerKernelRef,
    CHARIOX_SLICE_DAEMON_ALIAS: `slice:${fixture.localName}`,
    CHARIOX_SLICE_MACHINE_ID: fixture.machineId,
  }
  for (const action of ["provision", "recover"]) {
    const result = validate({ kind: "provisioner", action, environment, files: [] }, root)
    assert.equal(result.status, 0, result.stderr)
    assert.equal(result.stdout.trim(), "ok")
  }
  assert.notEqual(validate({ kind: "provisioner", action: "stop", environment, files: [] }, root).status, 0)
})

test("broker start and unpause route Docker mutation through quota admission", async () => {
  const source = await readFile(broker, "utf8")
  const executeStart = source.indexOf("async function execute(request)")
  const executeEnd = source.indexOf("\nfunction errorResponse", executeStart)
  assert.notEqual(executeStart, -1)
  assert.notEqual(executeEnd, -1)
  const executeSource = source.slice(executeStart, executeEnd)

  assert.match(source, /import\s*\{[^}]*\brunWithSliceDiskQuotaAdmission\b[^}]*\}\s*from "\.\/slice-disk-quota-admission\.mjs"/)
  assert.match(executeSource, /const runPrepared = async \(admission\) => \{[\s\S]*?prepareDocker\(request\.args\)[\s\S]*?return spawnBounded\(command, args/)
  assert.match(executeSource, /const isDockerStartOrUnpause = request\.kind === "docker" && \["start", "unpause"\]/)
  assert.match(executeSource, /const result = isDockerStartOrUnpause\s*\? await sliceDiskQuotaCoordinator\.withContainerLock\(containerName, async \(lock\) => runWithSliceDiskQuotaAdmission\(\{[\s\S]*?quotaMarkerPresent: diskQuotaMarkerPresent\(containerName\),[\s\S]*?run: async \(quotaResult, admission\) => \{[\s\S]*?assertLockHeld\(lock\)[\s\S]*?const started = await runPrepared\(admission\)/)
})

test("home archive identities cannot select the shared state or artifact parent", async context => {
  const root = await mkdtemp(join(tmpdir(), "chariox-archive-identity-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  for (const id of [".", ".."]) for (const scope of ["state", "backup"]) {
    for (const kind of ["home_archive_capture", "home_archive_remove"]) {
      const request = { kind, id, scope,
        ...(kind === "home_archive_capture" ? { container: "chariox-slice-dev-home-archive-1" } : {}),
      }
      assert.notEqual(validate(request, root).status, 0, `${kind} must refuse ${scope}/${id}`)
    }
  }
})

test("snapshot helpers require bounded isolated resources and matching ownership", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-helper-policy-"))
  context.after(() => rm(root, {recursive: true, force: true}))
  const helper = "chariox-slice-dev-disk-admission-0123456789abcdef"
  const args = ["create", "--name", helper, "--memory", "512m", "--cpus", "1",
    "--pids-limit", "64", "--network", "none", "--label", `io.chariox.snapshot-helper=${helper}`,
    "--user", "root", "-v", "chariox-slice-dev-home:/home-src:ro", "chariox-slice-linux:test", "sleep", "infinity"]
  assert.equal(validate({kind: "docker", args}, root).status, 0)
  for (const [index, value] of [[4, "0"], [6, "0"], [8, "0"], [10, "host"],
    [12, "io.chariox.snapshot-helper=another"], [16, "chariox-slice-other-home:/home-src:ro"],
    [16, "chariox-slice-dev-home:/home-src:rw"], [2, "chariox-slice-dev"]]) {
    const invalid = [...args]; invalid[index] = value
    assert.notEqual(validate({kind: "docker", args: invalid}, root).status, 0, `${index}: ${value}`)
  }
  assert.notEqual(validate({kind: "docker", args: [...args, "--privileged"]}, root).status, 0)
})

async function waitFor(check, timeoutMs = 3000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await check()) return
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20))
  }
  throw new Error("timed out waiting for broker state")
}

test("managed slice broker accepts only Chariox resources and shared host paths", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-test-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  const workspace = join(share, "slices/development/slice-dev/development/workspace")
  await mkdir(workspace, { recursive: true })

  assert.equal(validate({ kind: "docker", args: ["info"] }, share).status, 0)
  assert.equal(validate({
    kind: "docker",
    args: ["info", "--format", "{{.MemTotal}}"],
  }, share).status, 0)
  assert.equal(validate({
    kind: "docker",
    args: ["ps", "-a", "--format", "{{.Names}}"],
  }, share).status, 0)
  assert.equal(validate({
    kind: "docker",
    args: ["inspect", "--format", "{{.HostConfig.Memory}}", "chariox-slice-dev"],
  }, share).status, 0)
  assert.equal(validate({
    kind: "docker",
    args: ["inspect", "--size", "--format", "{{.SizeRw}}", "chariox-slice-dev"],
  }, share).status, 0)
  assert.equal(validate({
    kind: "docker",
    args: ["info", "--format", "{{.DockerRootDir}}"],
  }, share).status, 0)
  assert.equal(validate({
    kind: "docker",
    args: ["inspect", "--format", "{{json .Config}}", "chariox-slice-dev"],
  }, share).status, 1)
  assert.equal(validate({
    kind: "home_archive_capture",
    container: "chariox-slice-dev-home-archive-1",
    scope: "state",
    id: "chariox-slice-dev",
  }, share).status, 0)
  assert.equal(validate({
    kind: "home_archive_remove",
    scope: "backup",
    id: "chariox-slice-dev-backup-1",
  }, share).status, 0)
  assert.equal(validate({
    kind: "home_archive_verify",
    scope: "backup",
    id: "chariox-slice-dev-backup-1",
    path: join(share, ".broker-private/artifacts/backups/chariox-slice-dev-backup-1/generation-abcdef/home.tar.zst"),
  }, share).status, 0)
  assert.equal(validate({
    kind: "home_archive_capture",
    container: "chariox-slice-dev-home-archive-1",
    scope: "state",
    id: "../escape",
  }, share).status, 1)
  assert.equal(
    validate(
      {
        kind: "docker",
        args: [
          "create",
          "--name",
          "chariox-slice-dev-home-archive-1",
          "--memory", "512m",
          "--cpus", "1",
          "--pids-limit", "64",
          "--network", "none",
          "--label", "io.chariox.snapshot-helper=chariox-slice-dev-home-archive-1",
          "--user",
          "root",
          "-v",
          "chariox-slice-dev-home:/home-src:ro",
          "chariox-slice-linux:test",
          "sleep",
          "infinity",
        ],
      },
      share,
    ).status,
    0,
  )
  const diskHelper = "chariox-slice-dev-disk-admission-0123456789abcdef"
  for (const args of [
    ["exec", "-u", "root", diskHelper, "du", "-sb", "/home-src"],
    ["exec", "-u", "root", diskHelper, "bash", "-lc", "set -euo pipefail; find /home-src -printf . | wc -c"],
    ["exec", "-u", "root", diskHelper, "df", "-B1", "--output=avail", "/tmp"],
  ]) {
    assert.equal(validate({ kind: "docker", args }, share).status, 0)
  }
  assert.equal(validate({
    kind: "docker",
    args: ["exec", "-u", "root", diskHelper, "du", "-sb", "/etc"],
  }, share).status, 1)
  assert.equal(validate({
    kind: "docker",
    args: ["exec", "-u", "root", "chariox-slice-dev-home-archive-1", "bash", "-lc",
      "set -euo pipefail; cd /home-src; tar --zstd -cf /tmp/home.tar.zst ."],
  }, share).status, 1, "home archives must never be written into helper layers")
  assert.equal(validate({ kind: "home_archive_capture", container: "chariox-slice-dev",
    scope: "state", id: "chariox-slice-dev" }, share).status, 1, "only archive helpers may stream homes")
  const provision = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_HOSTNAME: "chariox-slice-dev-a1b2c3d4e5f6",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_OWNER_PUBLIC_KEY: ownerPublicKey,
        CHARIOX_SLICE_WORKSPACE: workspace,
      },
      files: [],
    },
    share,
  )
  assert.equal(provision.status, 0, provision.stderr)

  const recover = validate(
    {
      kind: "provisioner",
      action: "recover",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_HOSTNAME: "chariox-slice-dev-a1b2c3d4e5f6",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_OWNER_PUBLIC_KEY: ownerPublicKey,
        CHARIOX_SLICE_WORKSPACE: workspace,
        CHARIOX_SLICE_START_RUNTIME: "1",
        CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT: "1",
        CHARIOX_SLICE_DEVELOPMENT_MOUNT_0: workspace,
      },
      files: [],
    },
    share,
  )
  assert.equal(recover.status, 0, recover.stderr)

  const restoreState = validate(
    {
      kind: "provisioner",
      action: "restore-state",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_HOSTNAME: "chariox-slice-dev-a1b2c3d4e5f6",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_OWNER_PUBLIC_KEY: ownerPublicKey,
        CHARIOX_SLICE_WORKSPACE: workspace,
        CHARIOX_SLICE_SAVED_HOME_ARCHIVE: join(share, ".broker-private/artifacts/backups/chariox-slice-dev-backup-1/generation-abcdef/home.tar.zst"),
      },
      files: [],
    },
    share,
  )
  assert.equal(restoreState.status, 0, restoreState.stderr)

  const existingMixedCaseHostname = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-Production-1",
        CHARIOX_SLICE_HOSTNAME: "chariox-slice-Production-1",
        CHARIOX_SLICE_ID: "slice-production-1",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-Production-1-home",
        CHARIOX_SLICE_OWNER_PUBLIC_KEY: ownerPublicKey,
        CHARIOX_SLICE_WORKSPACE: join(share, "slices/development/slice-production-1/development/workspace"),
      },
      files: [],
    },
    share,
  )
  assert.equal(existingMixedCaseHostname.status, 0, existingMixedCaseHostname.stderr)

  const namedAppArmorProfile = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_OWNER_PUBLIC_KEY: ownerPublicKey,
        CHARIOX_SLICE_APPARMOR_PROFILE: "chariox-slice-provider",
        CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY: "1",
      },
      files: [],
    },
    share,
  )
  assert.equal(namedAppArmorProfile.status, 0, namedAppArmorProfile.stderr)

  const injectedAppArmorProfile = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_OWNER_PUBLIC_KEY: ownerPublicKey,
        CHARIOX_SLICE_APPARMOR_PROFILE: "unconfined --privileged",
      },
      files: [],
    },
    share,
  )
  assert.equal(injectedAppArmorProfile.status, 1)
  assert.match(injectedAppArmorProfile.stderr, /CHARIOX_SLICE_APPARMOR_PROFILE is invalid/)

  const malformedOwnerKey = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_OWNER_PUBLIC_KEY: "not-a-relay-public-key",
      },
      files: [],
    },
    share,
  )
  assert.equal(malformedOwnerKey.status, 1)
  assert.match(malformedOwnerKey.stderr, /relay owner public key is invalid/)

  for (const action of ["stop", "destroy"]) {
    const lifecycle = validate(
      {
        kind: "provisioner",
        action,
        environment: {
          CHARIOX_SLICE_NAME: "chariox-slice-dev",
          CHARIOX_SLICE_ID: "slice-dev",
          CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
          CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-dev",
          CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-dev",
        },
        files: [],
      },
      share,
    )
    assert.equal(lifecycle.status, 0, lifecycle.stderr)
  }

  const hostBind = validate(
    {
      kind: "docker",
      args: [
        "create",
        "--name",
        "chariox-slice-escape",
        "-v",
        "/home/chariox/.chariox/vault:/vault",
        "chariox-slice-linux:test",
      ],
    },
    share,
  )
  assert.equal(hostBind.status, 1)
  assert.match(hostBind.stderr, /Docker command shape is not allowed/)

  const arbitrary = validate({ kind: "docker", args: ["build", "."] }, share)
  assert.equal(arbitrary.status, 1)
  assert.match(arbitrary.stderr, /Docker command shape is not allowed/)
  const arbitraryExec = validate({
    kind: "docker",
    args: ["exec", "-u", "root", "chariox-slice-dev", "cat", "/etc/passwd"],
  }, share)
  assert.equal(arbitraryExec.status, 1)
  assert.match(arbitraryExec.stderr, /Docker exec command shape is not allowed/)

  const relayUrlRead = validate({
    kind: "docker",
    args: [
      "exec",
      "-u",
      "slice",
      "chariox-slice-dev",
      "jq",
      "-r",
      ".relay_url",
      "/home/slice/.chariox/daemon/config.json",
    ],
  }, share)
  assert.equal(relayUrlRead.status, 0, relayUrlRead.stderr)

  const runtimeLogs = [
    "exec",
    "-u",
    "slice",
    "chariox-slice-dev",
    "sh",
    "-c",
    sliceRuntimeLogScript,
    "slice-runtime-logs",
    "200",
    "legacy",
  ]
  const localDockerSource = await readFile(
    join(repositoryRoot, "apps/kernel/src/slice/local_docker.rs"),
    "utf8",
  )
  const canonicalRuntimeLogScript = localDockerSource.match(
    /fn local_docker_runtime_log_entry[\s\S]*?let script = r#"([\s\S]*?)"#;/,
  )?.[1]
  const brokerSource = await readFile(broker, "utf8")
  const brokerRuntimeLogScriptLiteral = brokerSource.match(
    /const SLICE_RUNTIME_LOG_SCRIPT = `([\s\S]*?)`/,
  )?.[1]
  const canonicalTemplateLiteral = sliceRuntimeLogScript
    .replaceAll("\\", "\\\\")
    .replaceAll("`", "\\`")
    .replaceAll("${", "\\${")
  assert.equal(canonicalRuntimeLogScript, sliceRuntimeLogScript)
  assert.equal(brokerRuntimeLogScriptLiteral, canonicalTemplateLiteral)
  assert.equal(validate({ kind: "docker", args: runtimeLogs }, share).status, 0)
  const protectedRuntimeLogs = [...runtimeLogs.slice(0, -1), "protected"]
  assert.equal(validate({ kind: "docker", args: protectedRuntimeLogs }, share).status, 0)
  assert.equal(validate({ kind: "docker", args: [...runtimeLogs.slice(0, -1), "/private/arbitrary"] }, share).status, 1)

  const injectedRuntimeLogs = validate({
    kind: "docker",
    args: runtimeLogs.map((argument, index) => (
      index === 6 ? `${argument}\nprintf owned >/tmp/broker-bypass` : argument
    )),
  }, share)
  assert.equal(injectedRuntimeLogs.status, 1)
  assert.match(injectedRuntimeLogs.stderr, /Docker exec command shape is not allowed/)

  for (const path of [
    "/home/slice/.chariox/daemon/provider-accounts/owner-1/codex/codex-1/codex/auth.json",
    "/home/slice/.chariox/daemon/provider-accounts/owner-1/opencode/opencode-1/data/opencode/auth.json",
  ]) {
    const accountCredential = validate({
      kind: "docker",
      args: ["exec", "-u", "slice", "chariox-slice-dev", "test", "-s", path],
    }, share)
    assert.equal(accountCredential.status, 0, accountCredential.stderr)
  }

  for (const path of [
    "/home/slice/.chariox/daemon/provider-accounts/owner-1/codex/codex-1/../../../../../../etc/shadow",
    "/home/slice/.chariox/daemon/provider-accounts/owner-1/codex/codex-1/unexpected",
    "/home/slice/.chariox/daemon/provider-accounts/owner-1/claude/claude-1/claude/.credentials.json",
  ]) {
    const accountCredentialEscape = validate({
      kind: "docker",
      args: ["exec", "-u", "slice", "chariox-slice-dev", "test", "-s", path],
    }, share)
    assert.equal(accountCredentialEscape.status, 1)
    assert.match(accountCredentialEscape.stderr, /Docker exec command shape is not allowed/)
  }
  const stopInjection = validate({
    kind: "provisioner",
    action: "stop",
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
      CHARIOX_SLICE_WORKSPACE: workspace,
    },
    files: [],
  }, share)
  assert.equal(stopInjection.status, 1)
  assert.match(stopInjection.stderr, /not allowed for provisioner action stop/)

  for (const args of [
    ["container", "create", "--name", "chariox-slice-escape", "--mount", "type=bind,source=/etc,target=/vault"],
    ["volume", "create", "--opt", "type=none", "--opt", "device=/etc", "chariox-slice-escape"],
    ["create", "--name", "chariox-slice-escape", "--mount", "type=bind,source=/etc,target=/vault"],
  ]) {
    const bypass = validate({ kind: "docker", args }, share)
    assert.equal(bypass.status, 1)
    assert.match(bypass.stderr, /Docker command shape is not allowed/)
  }

  for (const environment of [
    { CHARIOX_SLICE_DOCKER_IMAGE: "--privileged" },
    { CHARIOX_SLICE_BASE_IMAGE: "--mount=type=bind,source=/etc,target=/vault" },
    { CHARIOX_SLICE_BUILD_IMAGE: "sometimes" },
    { CHARIOX_SLICE_DOCKER_MEMORY: "1g --privileged" },
    { CHARIOX_SLICE_DOCKER_CPUS: "2 --volume=/etc:/vault" },
    { CHARIOX_SLICE_WORKSPACE_MOUNT_MODE: "rw,bind" },
    { CHARIOX_SLICE_HOSTNAME: "chariox_slice_dev" },
  ]) {
    const injected = validate({
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        ...environment,
      },
      files: [],
    }, share)
    assert.equal(injected.status, 1)
  }

  const extension = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_EXTENSION_DOCKERFILE: join(workspace, "Dockerfile"),
      },
      files: [],
    },
    share,
  )
  assert.equal(extension.status, 1)
})

test("managed slice broker verifies archive size and digest before restore", {
  skip: process.platform !== "linux" ? "managed broker pins files through Linux /proc" : false,
}, async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-archive-verify-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  const artifactRoot = join(share, ".broker-private/artifacts")
  const id = "backup-1"
  const generation = join(artifactRoot, "backups", id, "generation-abcdef")
  const archive = join(generation, "home.tar.zst")
  const contents = Buffer.from("verified archive")
  const digest = createHash("sha256").update(contents).digest("hex")
  await mkdir(generation, { recursive: true })
  await writeFile(archive, contents)
  await writeFile(join(generation, "metadata.json"), JSON.stringify({
    schemaVersion: 1,
    scope: "backup",
    id,
    sizeBytes: contents.length,
    sha256: digest,
  }))
  const request = JSON.stringify({ kind: "home_archive_verify", scope: "backup", id, path: archive })
  const run = () => spawnSync(process.execPath, [broker, "--stdio"], {
    input: `${request}\n`,
    encoding: "utf8",
    env: {
      ...process.env,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_BROKER_ARTIFACT_ROOT: artifactRoot,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })

  const valid = run()
  assert.equal(valid.status, 0, valid.stderr)
  const validResponse = JSON.parse(valid.stdout)
  assert.equal(validResponse.status, 0, Buffer.from(validResponse.stderrBase64, "base64").toString())
  assert.deepEqual(
    JSON.parse(Buffer.from(validResponse.stdoutBase64, "base64").toString()),
    { path: archive, sizeBytes: contents.length, sha256: digest },
  )

  await writeFile(archive, "corrupted archive")
  const corrupted = run()
  assert.equal(corrupted.status, 0, corrupted.stderr)
  const corruptedResponse = JSON.parse(corrupted.stdout)
  assert.notEqual(corruptedResponse.status, 0)
  assert.match(
    Buffer.from(corruptedResponse.stderrBase64, "base64").toString(),
    /digest does not match|metadata is invalid/,
  )
})

test("managed slice broker permits a disk-admission helper through its execution-time start gate", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-disk-helper-start-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  await mkdir(share)
  const run = (container) => spawnSync(process.execPath, [broker, "--stdio"], {
    input: `${JSON.stringify({ kind: "docker", args: ["start", container] })}\n`,
    encoding: "utf8",
    env: {
      ...process.env,
      DOCKER_HOST: `unix://${join(root, "unavailable-docker.sock")}`,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })

  const helper = run("chariox-slice-dev-disk-admission-0123456789abcdef")
  assert.equal(helper.status, 0, helper.stderr)
  const helperResponse = JSON.parse(helper.stdout)
  assert.notEqual(helperResponse.status, 0)
  assert.doesNotMatch(
    Buffer.from(helperResponse.stderrBase64, "base64").toString(),
    /no broker-owned stable mount record/,
  )

  const unowned = run("chariox-slice-dev-unowned-helper")
  assert.equal(unowned.status, 0, unowned.stderr)
  const unownedResponse = JSON.parse(unowned.stdout)
  assert.equal(unownedResponse.status, 125)
  assert.match(
    Buffer.from(unownedResponse.stderrBase64, "base64").toString(),
    /no broker-owned stable mount record/,
  )
})

test("managed broker preflight requires the same slice publication as execution", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-publication-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  await mkdir(share)
  const request = (workspace) => ({
    kind: "provisioner", action: "provision", files: [],
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
      CHARIOX_SLICE_WORKSPACE: workspace,
    },
  })
  const expected = join(share, "slices/development/slice-dev/development/workspace")
  await mkdir(expected, { recursive: true })
  assert.equal(validate(request(expected), share).status, 0)
  for (const relative of [
    "slices/development/slice-dev/empty-development/workspace",
    "slices/development/other-slice/development/workspace",
    "slices/development/slice-dev/development/.receipt",
  ]) {
    const workspace = join(share, relative)
    await mkdir(workspace, { recursive: true })
    const result = validate(request(workspace), share)
    assert.equal(result.status, 1, `preflight accepted invalid publication ${relative}`)
    assert.match(result.stderr, /direct repository in the matching slice publication/)
    const restore = validate({ ...request(workspace), action: "restore-state" }, share)
    assert.equal(restore.status, 1, `restore preflight accepted invalid publication ${relative}`)
    assert.match(restore.stderr, /direct repository in the matching slice publication/)
  }
})

test("managed slice broker rejects symlink escapes from the shared root", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-symlink-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  const outside = join(root, "outside")
  await mkdir(share)
  await mkdir(outside)
  await symlink(outside, join(share, "escape"))

  const result = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_WORKSPACE: join(share, "escape"),
      },
      files: [],
    },
    share,
  )
  assert.equal(result.status, 1)
  assert.match(result.stderr, /resolves outside|symbolic link/)
})

test("managed slice broker projects the signed context digest and gates build proofs on layout trust", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-digest-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  await mkdir(share)
  const digest = `sha256:${"a".repeat(64)}`
  const manifest = join(root, "release-manifest.json")
  await writeFile(manifest, JSON.stringify({
    artifacts: [{
      name: "chariox-slice-build-context",
      path: "/usr/lib/chariox/slice-build-context",
      sha256: digest,
    }],
  }))
  const provisioner = join(root, "provisioner.sh")
  await writeFile(provisioner, `#!${process.execPath}
process.stdout.write(JSON.stringify({digest: process.env.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST,
 proofRoot: process.env.CHARIOX_SLICE_PROTECTED_IMAGE_PROOF_ROOT}))
`)
  await chmod(provisioner, 0o755)
  const request = {
    kind: "provisioner",
    // This owned fake only prints its environment. Quota provisioning is
    // exercised separately; this fixture must not invoke real Docker.
    action: "stop",
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
      CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-dev",
      CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-dev",
    },
    files: [],
  }
  for (const dockerHost of ["unix:///run/chariox-docker/docker.sock", "unix:///synthetic/untrusted.sock"]) {
    const result = spawnSync(process.execPath, [broker, "--stdio"], {
      input: `${JSON.stringify(request)}\n`,
      encoding: "utf8",
      env: {
        ...process.env,
        DOCKER_HOST: dockerHost,
        CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
        CHARIOX_MANAGED_RELEASE_MANIFEST: manifest,
        CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
        CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
        CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
      },
    })
    assert.equal(result.status, 0, result.stderr)
    const response = JSON.parse(result.stdout)
    assert.equal(response.status, 0, Buffer.from(response.stderrBase64, "base64").toString())
    const environment = JSON.parse(Buffer.from(response.stdoutBase64, "base64").toString())
    assert.equal(environment.digest, digest)
    const trusted = process.platform === "linux" && process.getuid() === 0 && dockerHost === "unix:///run/chariox-docker/docker.sock"
    assert.equal(environment.proofRoot !== undefined, trusted)
  }

})

test("managed slice broker materializes bounded credential bytes privately", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-credentials-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  const inputRoot = join(root, "broker-input")
  await mkdir(share)
  const provisioner = join(root, "provisioner.sh")
  await writeFile(provisioner, `#!${process.execPath}
const { readFileSync, statSync } = require("node:fs")
const credential = process.env.CHARIOX_SLICE_GITHUB_TOKEN_FILE
const mode = statSync(credential).mode & 0o777
if (mode !== 0o600) {
  console.error("credential mode is " + mode.toString(8) + ", expected 600")
  process.exit(1)
}
process.stdout.write(readFileSync(credential))
`)
  await chmod(provisioner, 0o755)
  const request = {
    kind: "provisioner",
    action: "import-provider-auth",
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
    },
    files: [{
      environment: "CHARIOX_SLICE_GITHUB_TOKEN_FILE",
      name: "github-token.txt",
      contentsBase64: Buffer.from("credential-bytes").toString("base64"),
    }],
  }
  // A separate process serves Docker's exact missing-owned-container response.
  // The broker's synchronous control call must not depend on this test loop,
  // a real daemon, or an operational error being interpreted as absence.
  const dockerSocket = join(root, "docker.sock")
  const fixture = spawn(process.execPath, ["--input-type=module", "-e", `
import http from "node:http"
const server = http.createServer((request, response) => {
  response.setHeader("API-Version", "1.47")
  if (request.url === "/_ping") {
    response.end("OK")
  } else if (/^\\/(?:v[0-9.]+\\/)?containers\\/chariox-slice-dev\\/json$/.test(request.url)) {
    response.writeHead(404, { "Content-Type": "application/json" })
    response.end(JSON.stringify({ message: "No such container: chariox-slice-dev" }))
  } else {
    response.writeHead(500, { "Content-Type": "application/json" })
    response.end(JSON.stringify({ message: "unexpected synthetic Docker request" }))
  }
})
server.listen(process.argv[1], () => process.stdout.write("ready\\n"))
`, dockerSocket], { stdio: ["ignore", "pipe", "pipe"] })
  let result
  try {
    const [ready] = await once(fixture.stdout, "data", { signal: AbortSignal.timeout(3000) })
    assert.equal(ready.toString(), "ready\n")
    result = spawnSync(process.execPath, [broker, "--stdio"], {
      input: `${JSON.stringify(request)}\n`,
      encoding: "utf8",
      timeout: 5000,
      killSignal: "SIGKILL",
      env: {
        ...process.env,
        DOCKER_HOST: `unix://${dockerSocket}`,
        CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
        CHARIOX_SLICE_DOCKER_BROKER_INPUT_ROOT: inputRoot,
        CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
        CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
        CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
      },
    })
  } finally {
    if (fixture.exitCode === null && fixture.signalCode === null) {
      const closed = once(fixture, "close")
      fixture.kill("SIGTERM")
      await closed
    }
  }
  const response = JSON.parse(result.stdout)
  const brokerStderr = Buffer.from(response.stderrBase64, "base64").toString()
  assert.equal(result.status, 0, `${result.stderr}${brokerStderr}`)
  assert.equal(response.status, 0, brokerStderr)
  assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), "credential-bytes")
  assert.deepEqual(await access(inputRoot).then(() => true, () => false), true)
  assert.deepEqual(await import("node:fs/promises").then(({ readdir }) => readdir(inputRoot)), [])

  const injected = validate({
    ...request,
    environment: { ...request.environment, CHARIOX_SLICE_GITHUB_TOKEN_FILE: "/etc/passwd" },
    files: [],
  }, share)
  assert.equal(injected.status, 1)
})

async function processesInMountNamespace(namespace) {
  const entries = await readdir("/proc")
  const matches = await Promise.all(entries.filter((entry) => /^\d+$/.test(entry)).map(async (pid) => {
    const current = await readlink(`/proc/${pid}/ns/mnt`).catch(() => null)
    return current === namespace ? pid : null
  }))
  return matches.filter((pid) => pid !== null)
}

for (const abortBeforeRelease of [false, true]) test(abortBeforeRelease
  ? "managed slice broker fixture settles owned descendants after abort before release"
  : "managed slice broker pins a provisioner path inode across caller replacement", async (context) => {
  if (process.platform !== "linux" || process.env.CHARIOX_RUN_PRIVILEGED_MOUNT_TESTS !== "1") {
    context.skip("requires an explicitly enabled Linux mount namespace")
    return
  }
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-pin-"))
  const share = join(root, "share")
  const workspace = join(share, "slices/development/slice-dev/development/workspace")
  const moved = join(share, "workspace-original")
  const outside = join(root, "outside")
  const started = join(root, "started")
  const release = join(root, "release")
  const quotaRoot = join(root, "quota")
  const isolatedRun = join(root, "run")
  const handle = createHash("sha256").update("chariox-slice-dev\0workspace").digest("hex")
  const target = join(root, "handles", handle)
  let child
  let namespace
  context.after(async () => {
    if (child?.exitCode === null && child.signalCode === null) {
      child.kill("SIGTERM")
      const settled = () => Promise.resolve(child.exitCode !== null || child.signalCode !== null)
      try {
        await waitFor(settled, 1000)
      } catch {
        child.kill("SIGKILL")
        await waitFor(settled)
      }
    }
    if (namespace) await waitFor(async () => (await processesInMountNamespace(namespace)).length === 0)
    // The child namespace owns the bind mount; exiting it must leave no host mount.
    assert.notEqual(spawnSync("/usr/bin/mountpoint", ["-q", "--", target]).status, 0)
    await rm(root, { recursive: true, force: true })
  })
  await mkdir(workspace, { recursive: true })
  await mkdir(outside)
  await mkdir(quotaRoot, { mode: 0o700 })
  await mkdir(isolatedRun, { mode: 0o700 })
  await writeFile(join(quotaRoot, "reservations.json"), JSON.stringify({
    schemaVersion: 1, nextProjectId: 1073741824, reservations: {},
  }), { mode: 0o600 })
  await writeFile(join(workspace, "value"), "safe")
  await writeFile(join(outside, "value"), "outside-fixture")
  const fakeDocker = join(root, "docker.sh")
  await writeFile(fakeDocker, `#!/bin/sh
set -eu
case "$*" in
  'container inspect --format {{json .Config.Labels}} chariox-slice-dev'|'container inspect --format {{json .Mounts}} chariox-slice-dev')
    printf 'Error: No such container: chariox-slice-dev\\n' >&2; exit 1 ;;
  'volume inspect --format {{json .Labels}} chariox-slice-dev-home')
    printf 'Error: No such volume: chariox-slice-dev-home\\n' >&2; exit 1 ;;
  *) printf 'unexpected Docker operation: %s\\n' "$*" >&2; exit 99 ;;
esac
`)
  await chmod(fakeDocker, 0o755)
  const namespaceEntrypoint = join(root, "namespace.sh")
  await writeFile(namespaceEntrypoint, `#!/bin/sh
set -eu
mount --bind "$1" /usr/bin/docker
mount --bind "$2" /var/lib/chariox-slice-disk-quota
mount --bind "$3" /run
exec "$4" "$5" --stdio
`)
  await chmod(namespaceEntrypoint, 0o755)
  const provisioner = join(root, "provisioner.sh")
  await writeFile(provisioner, `#!/bin/sh
set -eu
mountpoint -q -- "$CHARIOX_SLICE_WORKSPACE_SOURCE"
: > '${started}'
while [ ! -e '${release}' ]; do sleep 0.01; done
# WORKSPACE is the container destination; SOURCE is the broker's pinned bind mount.
cat "$CHARIOX_SLICE_WORKSPACE_SOURCE/value"
`)
  await chmod(provisioner, 0o755)
  child = spawn("/usr/bin/unshare", [
    // Killing unshare kills the namespace init, so separate timeout process
    // groups cannot outlive fixture teardown or retain privileged mounts.
    "--mount", "--propagation", "private", "--pid", "--fork", "--kill-child=SIGKILL", "--mount-proc", namespaceEntrypoint,
    fakeDocker, quotaRoot, isolatedRun, process.execPath, broker,
  ], {
    detached: true,
    stdio: ["pipe", "pipe", "pipe"],
    env: {
      ...process.env,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })
  let stdout = ""
  let stderr = ""
  child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk })
  child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk })
  child.stdin.write(`${JSON.stringify({
    kind: "provisioner",
    action: "provision",
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
      CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-dev",
      CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-dev",
      CHARIOX_SLICE_WORKSPACE: workspace,
    },
    files: [],
  })}\n`)
  await waitFor(async () => await access(started).then(() => true, () => false) || stdout.includes("\n"))
  const failure = stdout.includes("\n") ? Buffer.from(JSON.parse(stdout).stderrBase64, "base64").toString() : stderr
  assert.equal(await access(started).then(() => true, () => false), true, failure)
  namespace = await readlink(`/proc/${child.pid}/ns/mnt`)
  assert.notEqual(namespace, await readlink("/proc/self/ns/mnt"))
  const owned = await processesInMountNamespace(namespace)
  const commands = await Promise.all(owned.map((pid) => readFile(`/proc/${pid}/cmdline`, "utf8").catch(() => "")))
  assert.ok(commands.some((command) => command.includes("slice-command-guard.py")), "fixture reached the common process-group owner")
  if (abortBeforeRelease) return // Exercise teardown while the provisioner waits in its separate process group.
  await rename(workspace, moved)
  await symlink(outside, workspace)
  assert.equal(await readFile(join(workspace, "value"), "utf8"), "outside-fixture")
  await writeFile(release, "go")
  await waitFor(() => Promise.resolve(stdout.includes("\n")))
  child.stdin.end()
  await once(child, "exit")
  assert.equal(stderr, "")
  const response = JSON.parse(stdout)
  assert.equal(response.status, 0, Buffer.from(response.stderrBase64, "base64").toString())
  assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), "safe")
})

test("managed slice broker gives a slice without a workspace its own volume, not the build context", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-owned-workspace-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  await mkdir(share)
  const provisioner = join(root, "provisioner.sh")
  await writeFile(provisioner, "#!/bin/sh\nprintf '%s' \"${CHARIOX_SLICE_OWNED_WORKSPACE:-unset}\"\n")
  await chmod(provisioner, 0o755)
  const dockerHost = await missingBrokerDockerFixture(context, root, "chariox-slice-dev")
  const result = spawnSync(process.execPath, [broker, "--stdio"], {
    input: `${JSON.stringify({
      kind: "provisioner",
      // The broker projects the owned workspace for every provisioner action.
      // Quota provisioning is exercised separately; this owned fake must not
      // reach the quota allocator or real Docker.
      action: "stop",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
        CHARIOX_SLICE_ID: "slice-dev",
        CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
        CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-dev",
        CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-dev",
      },
      files: [],
    })}\n`,
    encoding: "utf8",
    env: {
      ...process.env,
      DOCKER_HOST: dockerHost,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })
  assert.equal(result.status, 0, result.stderr)
  const response = JSON.parse(result.stdout.trim())
  assert.equal(response.status, 0, Buffer.from(response.stderrBase64, "base64").toString())
  assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), "1")
})

test("MP-08 MP-11 broker verbose success preserves the next request", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-output-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  await mkdir(share)
  const provisioner = join(root, "provisioner.sh")
  await writeFile(provisioner, `#!/bin/sh
if [ -e "\${0}.once" ]; then printf valid; else touch "\${0}.once"; head -c 5242880 /dev/zero; fi
`)
  await chmod(provisioner, 0o755)
  const base = {
    kind: "provisioner",
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
      CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-dev",
      CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-dev",
    },
    files: [],
  }
  const dockerHost = await missingBrokerDockerFixture(context, root, "chariox-slice-dev")
  const result = spawnSync(process.execPath, [broker, "--stdio"], {
    input: `${JSON.stringify({ ...base, action: "stop" })}\n${JSON.stringify({ ...base, action: "stop" })}\n`,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    env: {
      ...process.env,
      DOCKER_HOST: dockerHost,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })
  assert.equal(result.status, 0, result.stderr)
  const responses = result.stdout.trim().split("\n").map(JSON.parse)
  assert.equal(responses.length, 2)
  assert.equal(responses[0].status, 0, Buffer.from(responses[0].stderrBase64, "base64").toString())
  assert(Buffer.from(responses[0].stdoutBase64,"base64").length <= 65536)
  assert.match(Buffer.from(responses[0].stderrBase64,"base64").toString(),/complete diagnostics/)
  assert.equal(responses[1].status, 0, Buffer.from(responses[1].stderrBase64, "base64").toString())
  assert.equal(Buffer.from(responses[1].stdoutBase64, "base64").toString(), "valid")
})

test("managed slice broker removes its endpoint after the supervisor claims it", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-lease-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  const socketPath = join(root, "control.sock")
  const outputRoot = join(share, ".broker-private/output")
  const artifactRoot = join(share, ".broker-private/artifacts")
  await mkdir(share)
  await mkdir(outputRoot, { recursive: true })
  await mkdir(artifactRoot, { recursive: true })
  const child = spawn(process.execPath, [broker], {
    stdio: ["ignore", "ignore", "pipe"],
    env: {
      ...process.env,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_BROKER_SOCKET: socketPath,
      CHARIOX_SLICE_DOCKER_BROKER_INPUT_ROOT: join(root, "input"),
      CHARIOX_SLICE_DOCKER_BROKER_OUTPUT_ROOT: outputRoot,
      CHARIOX_SLICE_DOCKER_BROKER_ARTIFACT_ROOT: artifactRoot,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })
  context.after(() => child.kill())
  await waitFor(async () => access(socketPath).then(() => true, () => false))
  const lease = createConnection(socketPath)
  await once(lease, "connect")
  await waitFor(async () => access(socketPath).then(() => false, () => true))
  const rejected = createConnection(socketPath)
  const [error] = await once(rejected, "error")
  assert.equal(error.code, "ENOENT")
  lease.end()
  const [status] = await once(child, "exit")
  assert.equal(status, 0)
})


test("runtime log script selects protected and legacy roots without mixing them", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-log-layout-"))
  try {
    for (const directory of ["protected-runtime", "protected-kernel", "legacy-runtime", "legacy-kernel"]) {
      await mkdir(join(root, directory))
      await writeFile(join(root, directory, directory.endsWith("runtime") ? "synthetic.log" : "synthetic.ndjson"), directory + "\n")
    }
    const script = sliceRuntimeLogScript
      .replaceAll("/var/lib/chariox/slice-private/runtime/logs", join(root, "protected-runtime"))
      .replaceAll("/var/lib/chariox/slice-private/kernel/logs", join(root, "protected-kernel"))
      .replaceAll("/opt/chariox-slice/logs", join(root, "legacy-runtime"))
      .replaceAll("/home/slice/.local/state/chariox/logs", join(root, "legacy-kernel"))
    for (const layout of ["protected", "legacy"]) {
      const result = spawnSync("sh", ["-c", script, "slice-runtime-logs", "200", layout], {encoding: "utf8"})
      assert.equal(result.status, 0, result.stderr)
      assert.match(result.stdout, new RegExp(layout + "-runtime"))
      assert.match(result.stdout, new RegExp(layout + "-kernel"))
      assert(!result.stdout.includes(layout === "protected" ? "legacy-" : "protected-"))
    }
  } finally { await rm(root, {recursive: true, force: true}) } // Only newly created synthetic logs.
})


test("MP-08 MP-10 MP-11 all provisioner requests use common owned lifetime", async () => {
  const source = await readFile(broker, "utf8")
  const calls = []
  const brokerLifetime = new AbortController()
  let prepared = 0
  let cleaned = 0
  const execute = brokerExecutionFixture(source, {
    ...archivePolicy, process, Buffer, Set,
    PROVISIONER: process.execPath, DOCKER_HOST: "unix:///synthetic/unused", MAX_OUTPUT_BYTES: 1024,
    // Phase 1 managed authority: no Local DEV enrollment, no verified build
    // context, and an accepting protected-layout controller.
    LOCAL_AUTHORITY: undefined, VERIFIED_BUILD_CONTEXT_DIGEST: "", protectedLayouts: { homeVolume: name => `${name}-home`, retainedHomeVolumes: () => [], complete: () => {} },
    validateRequest: () => {},
    provisionerQuotaRequest: () => ({ identity: { containerName: "synthetic-owned" } }),
    diskQuotaMarkerPresent: () => false,
    requestSliceDiskQuota: async () => ({ bounded: false }),
    exactKeys: () => {},
    sliceDiskQuotaCoordinator: {
      withContainerLock: async (_name, run) => run({}),
      assertUnbounded: async () => undefined,
    },
    prepareProvisioner: async request => { await new Promise(resolve => setImmediate(resolve)); prepared++; return { environment: request.environment, handles: new Set(), newHandles: new Set() } },
    cleanupPrepared: () => { cleaned++ }, removePersistentHandles: () => {},
    brokerLifetime, BROKER_OUTPUT_ROOT: "/synthetic/logs", join,
    runBrokerCommand: async (command, args, options) => {
      assert.equal(options.signal, brokerLifetime.signal, "commands share the broker lifetime cancellation signal")
      assert.equal(options.logRoot, "/synthetic/logs/logs", "commands use broker-owned output logs")
      calls.push({ command, args, timeout: options.timeout, env: options.env })
      return spawnSync(command, ["-e", "setTimeout(() => process.stdout.write('restored'), 120)"], options)
    },
  })
  for (const action of ["provision", "restore-state"]) {
    const response = await execute({ kind: "provisioner", action, files: [], environment: {
      CHARIOX_SLICE_NAME: "synthetic-owned", CHARIOX_SLICE_SAVED_HOME_ARCHIVE: "/private/synthetic/home.tar.zst",
    } })
    assert.equal(response.status, 0, `${action} must let the healthy restore complete`)
    assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), "restored")
    assert.equal(calls.at(-1).command, process.execPath)
    assert.equal(calls.at(-1).timeout, undefined)
    assert.equal(calls.at(-1).env.CHARIOX_SLICE_BROKER_BUILD_TIMEOUT_SECONDS, undefined)
  }
  for (const [action, archive] of [["provision", undefined], ["restore-state", ""], ["recover", "/private/home"], ["status", "/private/home"], ["import-provider-auth", "/private/home"]]) {
    const response = await execute({ kind: "provisioner", action, files: [], environment: {
      CHARIOX_SLICE_NAME: "synthetic-owned", ...(archive !== undefined ? { CHARIOX_SLICE_SAVED_HOME_ARCHIVE: archive } : {}),
    } })
    assert.equal(response.status, 0, `${action}/${archive} uses common owned lifetime`)
    assert.equal(calls.at(-1).command, process.execPath)
    assert.equal(calls.at(-1).timeout, undefined)
  }
  assert.equal(prepared, 7)
  assert.equal(cleaned, prepared, "prepared filesystem handles settle on each completed request")
})

test("Phase 1 local DEV broker runs unbounded slices without release F's managed quota coordination", async () => {
  const source = await readFile(broker, "utf8")
  // A local DEV host has no quota allocator, coordination root or admission
  // proofs: any use of them is a failure of this topology.
  const quotaUse = []
  const managedOnly = name => () => { quotaUse.push(name); throw new Error(`${name} exists only on managed hosts`) }
  const markers = new Set()
  const commands = []
  const released = []
  let authorityChecks = 0
  const execute = brokerExecutionFixture(source, {
    ...archivePolicy, process, Buffer, Set,
    PROVISIONER: process.execPath, DOCKER_HOST: "unix:///run/docker.sock", MAX_OUTPUT_BYTES: 1024,
    LOCAL_AUTHORITY: { enrollment: { ownerUid: 1000 } },
    verifiedProtectedAuthority: () => { authorityChecks++ },
    localDevRuntimeEnvironment: () => ({CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY: "1"}),
    VERIFIED_BUILD_CONTEXT_DIGEST: "", protectedLayouts: { homeVolume: name => `${name}-home`, retainedHomeVolumes: () => [], complete: () => {} },
    validateRequest: () => {},
    fail: message => { throw new Error(message) },
    diskQuotaMarkerPresent: name => markers.has(name),
    provisionerQuotaRequest: managedOnly("provisionerQuotaRequest"),
    requestSliceDiskQuota: managedOnly("requestSliceDiskQuota"),
    runWithSliceDiskQuotaAdmission: managedOnly("runWithSliceDiskQuotaAdmission"),
    sliceDiskQuotaIdentityFromEnvironment: managedOnly("sliceDiskQuotaIdentityFromEnvironment"),
    sliceDiskQuotaCoordinator: new Proxy({}, { get: (_target, name) => managedOnly(`sliceDiskQuotaCoordinator.${String(name)}`) }),
    prepareProvisioner: async request => ({ environment: request.environment, descriptors: [], handles: new Set(), newHandles: new Set() }),
    prepareDocker: args => ({ args: [...args], descriptors: [] }),
    dockerControlPolicy: () => ({}),
    dockerEnvironment: () => ({ PATH: "/usr/bin:/bin" }),
    recordedContainerMounts: () => [{ destination: "/workspace", rw: true, source: "/synthetic/handle" }],
    requireExactContainerMounts: () => true,
    isDiskAdmissionHelper: () => false,
    publishStagedOutput: () => {}, releasePersistentHandles: name => { released.push(name) },
    cleanupPrepared: () => {}, removePersistentHandles: () => {},
    brokerLifetime: new AbortController(), BROKER_OUTPUT_ROOT: "/synthetic/logs", join,
    runBrokerCommand: async (command, args, options) => {
      commands.push([command, ...args])
      if (command === process.execPath) assert.equal(options.env.CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY, "1", "enrolled DEV grant overrides the kernel default")
      return spawnSync(process.execPath, ["-e", "process.stdout.write('ran')"], options)
    },
  })
  const slice = "chariox-slice-local"
  const requests = [
    ...["provision", "restore-state", "recover", "destroy"].map(action => ({ kind: "provisioner", action, files: [], environment: { CHARIOX_SLICE_NAME: slice, CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY: "0" } })),
    { kind: "docker", args: ["start", slice] },
    { kind: "docker", args: ["unpause", slice] },
  ]
  for (const request of requests) {
    const response = await execute(request)
    const label = request.kind === "docker" ? request.args[0] : request.action
    assert.equal(response.status, 0, `${label} runs without managed quota coordination`)
    assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), "ran")
  }
  assert.deepEqual(commands.map(command => command.at(-1)), ["provision", "restore-state", "recover", "destroy", slice, slice])
  assert.deepEqual(released, [slice], "destroy still releases the slice's stable handles")
  assert.deepEqual(quotaUse, [])

  // Without an allocator the local broker can neither reserve quotas nor admit
  // a slice that carries a quota marker: both fail before any command runs.
  await assert.rejects(execute({ kind: "provisioner", action: "provision", files: [], environment: {
    CHARIOX_SLICE_NAME: slice, CHARIOX_SLICE_DISK_LAYER_MB: "1024", CHARIOX_SLICE_DISK_HOME_MB: "2048",
  } }), /do not support managed disk quotas/)
  markers.add("chariox-slice-marked")
  for (const request of [
    { kind: "docker", args: ["start", "chariox-slice-marked"] },
    { kind: "docker", args: ["unpause", "chariox-slice-marked"] },
    { kind: "provisioner", action: "recover", files: [], environment: { CHARIOX_SLICE_NAME: "chariox-slice-marked" } },
  ]) {
    await assert.rejects(execute(request), /cannot run without the managed quota allocator/)
  }
  assert.equal(commands.length, requests.length)
  assert.deepEqual(quotaUse, [])
  assert.equal(authorityChecks, requests.length + 4, "every local request re-verifies its enrolled authority")
})


test("broker control commands settle a signal-resistant process and allow the next control", async () => {
  const source = await readFile(broker, "utf8")
  const helper = source.slice(source.indexOf("function spawnControl("), source.indexOf("function handleIsMountpoint("))
  assert.ok(helper, "broker controls must have a bounded execution owner")
  const calls = []
  const control = runInNewContext(`${helper}\nspawnControl`, {
    spawnSync: (command, args, options) => {
      calls.push({ command, args, options })
      assert.equal(options.timeout, args.includes("stop") ? 30_000 : 20_000)
      assert.equal(options.killSignal, "SIGKILL")
      return spawnSync(process.execPath, ["-e", args.includes("stop")
        ? "process.on('SIGTERM',()=>{}); process.stderr.write('armed'); setInterval(()=>{},1000)"
        : "process.stdout.write('next-control')"], { ...options, timeout: 200 })
    },
  })
  const stopped = control("/usr/bin/docker", ["stop", "owned"], { timeout: 30_000, encoding: "utf8" })
  assert.equal(stopped.error?.code, "ETIMEDOUT")
  assert.equal(stopped.signal, "SIGKILL")
  assert.equal(stopped.stderr, "armed", "resistant producer must have started before cancellation")
  const next = control("/usr/bin/docker", ["container", "inspect", "owned"], { encoding: "utf8" })
  assert.equal(next.status, 0)
  assert.equal(next.stdout, "next-control")
  assert.equal(calls.length, 2)
})

test("broker mount admission refuses interrupted inspection and bounds stop before mutation", async () => {
  const source = await readFile(broker, "utf8")
  const inspect = source.slice(source.indexOf("function inspectContainerMounts("), source.indexOf("function diskQuotaMarkerPresent("))
  const exact = source.slice(source.indexOf("function requireExactContainerMounts("), source.indexOf("function stagedSharedOutput("))
  let result = { status: null, signal: "SIGKILL", error: { code: "ETIMEDOUT" } }
  const calls = []
  const check = runInNewContext(`${inspect}\n${exact}\nrequireExactContainerMounts`, {
    spawnControl: (command, args, options) => { calls.push({ command, args, options }); return result },
    dockerEnvironment: () => ({}), MAX_OUTPUT_BYTES: 1024,
    dockerObjectNotFound, normalizedMounts: mounts => mounts, fail: message => { throw new Error(message) },
  })
  assert.throws(() => check("owned", [], true), /mount inspection failed/)
  assert.equal(calls.length, 1, "timed out inspection cannot proceed to stop or mount mutation")
  for (const stderr of ["Cannot connect to the Docker daemon", "permission denied", "Error: No such object: other", "Error: No such object: owned\npermission denied"]) {
    result = { status: 1, stderr }
    assert.throws(() => check("owned", [], true), /mount inspection failed/)
  }
  result = { status: 1, stderr: "Error: No such object: owned\n" }
  assert.equal(check("owned", [], true), false, "only exact confirmed absence can skip stop")
  result = { status: 0, stdout: "[]" }
  assert.equal(check("owned", [], true), true)
  assert.deepEqual(Array.from(calls.at(-1).args), ["stop", "owned"])
  assert.equal(calls.at(-1).options.timeout, 30_000)
  for (const name of ["handleIsMountpoint", "unmountHandle", "publishHandle", "diskQuotaMarkerPresent", "unboundedQuotaObservation"]) {
    const body = source.slice(source.indexOf(`function ${name}(`), source.indexOf("\nfunction ", source.indexOf(`function ${name}(`) + 1))
    assert.match(body, /spawnControl\(/, `${name} must use bounded control execution`)
    assert.doesNotMatch(body, /spawnSync\(/)
  }
})


test("broker mountpoint errors cannot become permission to remove a mounted handle", async () => {
  const source = await readFile(broker, "utf8")
  const body = source.slice(source.indexOf("function handleIsMountpoint("), source.indexOf("function unmountHandle("))
  let result
  const isMountpoint = runInNewContext(`${body}\nhandleIsMountpoint`, {
    spawnControl: () => result, fail: message => { throw new Error(message) },
  })
  for (const status of [1, 2, 124, null]) {
    result = { status }
    assert.throws(() => isMountpoint("/owned/handle"), /failed to inspect/)
  }
  result = { status: 32, error: { code: "ETIMEDOUT" } }
  assert.throws(() => isMountpoint("/owned/handle"), /failed to inspect/)
  result = { status: 32, signal: "SIGKILL" }
  assert.throws(() => isMountpoint("/owned/handle"), /failed to inspect/)
  result = { status: 0 }
  assert.equal(isMountpoint("/owned/handle"), true)
  result = { status: 32 }
  assert.equal(isMountpoint("/owned/handle"), false)
})

test("request environment cannot override the broker build deadline", async context => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-build-policy-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  for (const value of ["0", "1", "1200", "999999"]) {
    const result = validate({ kind: "provisioner", action: "provision", files: [], environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev", CHARIOX_SLICE_BROKER_BUILD_TIMEOUT_SECONDS: value,
    } }, root)
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /environment/)
  }
})

test("MP-08/MP-11 broker CPU admission follows the shared positive Docker CPU cases", async context => {
  const root = await mkdtemp(join(tmpdir(), "chariox-cpu-policy-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const policy = JSON.parse(await readFile(join(repositoryRoot, "apps/kernel/slice-linux-docker/docker-cpu-policy.json"), "utf8"))
  for (const [cpus, accepted] of policy.cases) {
    const result = validate({ kind: "provisioner", action: "recover", environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev", CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home", CHARIOX_SLICE_DOCKER_CPUS: cpus,
    }, files: [] }, root)
    assert.equal(result.status === 0, accepted, `${JSON.stringify(cpus)}: ${result.stderr}`)
  }
})

test("MP-08 MP-10 MP-11 broker lifetime has no placement-selected total deadline", async () => {
 const source=await readFile(broker,"utf8")
 assert.doesNotMatch(source,/CHARIOX_SLICE_BROKER_BUILD_TIMEOUT_SECONDS|21 \* 60_000|20 \* 60_000|"20m"/)
 const provisioner=await readFile(join(repositoryRoot,"apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"),"utf8")
 assert.doesNotMatch(provisioner,/CHARIOX_SLICE_BROKER_BUILD_TIMEOUT_SECONDS/)
})

test("MP-08 MP-11 broker admits documented nondefault slice tuning", async context => {
 const root=await mkdtemp(join(tmpdir(),"chariox-broker-tuning-"))
 context.after(()=>rm(root,{recursive:true,force:true}))
 const environment={CHARIOX_SLICE_ID:"slice-dev",CHARIOX_SLICE_NAME:"chariox-slice-dev",CHARIOX_SLICE_HOME_VOLUME:"chariox-slice-dev-home",CHARIOX_SLICE_DOCKER_PIDS_LIMIT:"2048",CHARIOX_SLICE_DOCKER_NOFILE_LIMIT:"32768",CHARIOX_SLICE_MIN_FREE_MB:"768"}
 const result=validate({kind:"provisioner",action:"recover",environment,files:[]},root)
 assert.equal(result.status,0,result.stderr)
 for(const [name,values] of Object.entries({CHARIOX_SLICE_DOCKER_PIDS_LIMIT:["0","-1","2147483648","1.5"],CHARIOX_SLICE_DOCKER_NOFILE_LIMIT:["0","1023","1048577","bad"],CHARIOX_SLICE_MIN_FREE_MB:["-1","4294967296","bad"]})) {
  for(const value of values) assert.notEqual(validate({kind:"provisioner",action:"recover",environment:{...environment,[name]:value},files:[]},root).status,0,`${name} ${value}`)
 }
})

test("quota admission uses the retained generation and final evidence uses the prepared restore generation", async () => {
  const source = await readFile(broker, "utf8")
  const container = "chariox-slice-generation"
  const retained = `${container}-home-g${"a".repeat(32)}`
  const restored = `${container}-home-g${"b".repeat(32)}`
  const quotas = [], environments = []
  const execute = brokerExecutionFixture(source, {
    ...archivePolicy, process, Buffer, Set, join,
    PROVISIONER: "/synthetic/provisioner", DOCKER_HOST: "unix:///synthetic", MAX_OUTPUT_BYTES: 1024,
    LOCAL_AUTHORITY: undefined, VERIFIED_BUILD_CONTEXT_DIGEST: "", brokerLifetime: new AbortController(), BROKER_OUTPUT_ROOT: "/synthetic",
    protectedLayouts: {homeVolume: () => retained, complete: () => {}},
    validateRequest: () => {}, diskQuotaMarkerPresent: () => false,
    provisionerQuotaRequest: environment => ({identity: {containerName: container, homeVolumeName: environment.CHARIOX_SLICE_HOME_VOLUME}, limits: {persistentHomeBytes: 2048, writableLayerBytes: 1024}}),
    requestSliceDiskQuota: async request => {quotas.push(request); return {evidence: {checked: true}}},
    sliceDiskQuotaCoordinator: {withContainerLock: async (_name, run) => run({}), assertBounded: async () => {}},
    prepareProvisioner: async request => ({environment: {...request.environment, CHARIOX_SLICE_HOME_VOLUME: restored}, handles: new Set(), newHandles: new Set()}),
    runBrokerCommand: async (_command, _args, options) => {environments.push(options.env); return {status: 0, stdout: Buffer.from("ran"), stderr: Buffer.alloc(0)}},
    cleanupPrepared: () => {}, removePersistentHandles: () => {},
  })
  const result = await execute({kind: "provisioner", action: "restore-state", environment: {CHARIOX_SLICE_NAME: container, CHARIOX_SLICE_HOME_VOLUME: `${container}-home`}})
  assert.equal(result.status, 0)
  assert.deepEqual(quotas.map(q => [q.operation, q.identity.homeVolumeName]), [["reserve", retained], ["verify", restored]])
  assert.equal(environments[0].CHARIOX_SLICE_HOME_VOLUME, restored)
})

test("quota destroy retires retained generations after current-home removal even without a marker", async () => {
  const {retireProtectedQuotaHomes} = await import("../apps/kernel/slice-linux-docker/protected-home-retirement.mjs")
  const source = await readFile(broker, "utf8")
  const identity = {containerName: "chariox-slice-retirement", sliceId: "retirement", ownerKernelId: "kernel", ownerMachineId: "machine"}
  const oldHome = `${identity.containerName}-home`
  const currentHome = `${oldHome}-g${"e".repeat(32)}`
  identity.homeVolumeName = currentHome
  const remaining = new Set([oldHome, currentHome]), events = []
  const execute = brokerExecutionFixture(source, {
    ...archivePolicy, process, Buffer, Set, join,
    PROVISIONER: "/synthetic/provisioner", DOCKER_HOST: "unix:///synthetic", MAX_OUTPUT_BYTES: 1024,
    LOCAL_AUTHORITY: undefined, VERIFIED_BUILD_CONTEXT_DIGEST: "", brokerLifetime: new AbortController(), BROKER_OUTPUT_ROOT: "/synthetic",
    protectedLayouts: {homeVolume: () => currentHome, retainedHomeVolumes: () => [oldHome, currentHome]},
    validateRequest: () => {}, diskQuotaMarkerPresent: () => false,
    provisionerQuotaRequest: () => ({identity}), sliceDiskQuotaIdentityFromEnvironment: () => identity,
    requestSliceDiskQuota: async request => {
      events.push(request.operation)
      if (request.operation === "status") return {bounded: true}
      assert.equal(remaining.size, 0, "owned retained homes must be retired before quota release")
      return {released: true}
    },
    sliceDiskQuotaCoordinator: {withContainerLock: async (_name, run) => run({}), revokeUnboundedProof: () => {}},
    prepareProvisioner: async request => ({environment: request.environment, handles: new Set(), newHandles: new Set()}),
    runBrokerCommand: async () => {remaining.delete(currentHome); return {status: 0, stdout: Buffer.alloc(0), stderr: Buffer.alloc(0)}},
    spawnControl: (_command, args) => {
      if (!remaining.has(args[2])) return {status: 1, stderr: `Error: No such volume: ${args[2]}\n`}
      if (args[1] === "inspect") return {status: 0, stdout: JSON.stringify([{Name: args[2], Driver: "local", Labels: {
        "io.chariox.slice.id": identity.sliceId, "io.chariox.slice.owner-kernel-id": identity.ownerKernelId,
        "io.chariox.slice.owner-machine-id": identity.ownerMachineId,
      }}])}
      assert.equal(args[1], "rm"); remaining.delete(args[2]); events.push("retire"); return {status: 0}
    },
    retireProtectedQuotaHomes, dockerEnvironment: () => ({}),
    releasePersistentHandles: () => {}, cleanupPrepared: () => {}, removePersistentHandles: () => {},
  })
  const result = await execute({kind: "provisioner", action: "destroy", environment: {CHARIOX_SLICE_NAME: identity.containerName}})
  assert.equal(result.status, 0)
  assert.deepEqual(events, ["status", "retire", "release"])
})
