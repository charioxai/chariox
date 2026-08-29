import assert from "node:assert/strict"
import { access, chmod, mkdtemp, mkdir, readFile, rename, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { spawn, spawnSync } from "node:child_process"
import { once } from "node:events"
import { createConnection } from "node:net"
import { test } from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const broker = join(
  repositoryRoot,
  "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
)
const ownerPublicKey = Buffer.concat([Buffer.from([4]), Buffer.alloc(64, 1)]).toString("base64")
const sliceRuntimeLogScript = `
set -eu
found=0
for file in /opt/chariox-slice/logs/*.log /home/slice/.local/state/chariox/logs/*.ndjson; do
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
    env: { ...process.env, CHARIOX_SLICE_DOCKER_SHARE_ROOT: shareRoot },
  })
}

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
  const workspace = join(share, "managed-context/kernel/workspace")
  await mkdir(workspace, { recursive: true })

  assert.equal(validate({ kind: "docker", args: ["info"] }, share).status, 0)
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
          "chariox-slice-helper",
          "--user",
          "root",
          "-v",
          "chariox-slice-home:/home-src:ro",
          "chariox-slice-linux:test",
          "sleep",
          "infinity",
        ],
      },
      share,
    ).status,
    0,
  )
  const provision = validate(
    {
      kind: "provisioner",
      action: "provision",
      environment: {
        CHARIOX_SLICE_NAME: "chariox-slice-dev",
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
        "/var/lib/chariox/home/.chariox/vault:/vault",
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
  ]
  const localDockerSource = await readFile(
    join(repositoryRoot, "apps/kernel/src/slice/local_docker.rs"),
    "utf8",
  )
  const canonicalRuntimeLogScript = localDockerSource.match(
    /fn local_docker_runtime_log_entry[\s\S]*?let script = r#"([\s\S]*?)"#;/,
  )?.[1]
  assert.equal(canonicalRuntimeLogScript, sliceRuntimeLogScript)
  assert.equal(validate({ kind: "docker", args: runtimeLogs }, share).status, 0)
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
    "/home/slice/.chariox/daemon/provider-accounts/owner-1/claude/claude-1/claude/.credentials.json",
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

test("managed slice broker pins lazy builds to the signed context digest", async (context) => {
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
  await writeFile(provisioner, "#!/bin/sh\nprintf '%s' \"$CHARIOX_SLICE_BUILD_CONTEXT_DIGEST\"\n")
  await chmod(provisioner, 0o755)
  const request = {
    kind: "provisioner",
    action: "provision",
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
    },
    files: [],
  }
  const result = spawnSync(process.execPath, [broker, "--stdio"], {
    input: `${JSON.stringify(request)}\n`,
    encoding: "utf8",
    env: {
      ...process.env,
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
  assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), digest)
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
const credential = process.env.CHARIOX_SLICE_CODEX_AUTH
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
      environment: "CHARIOX_SLICE_CODEX_AUTH",
      name: "codex-auth.json",
      contentsBase64: Buffer.from("credential-bytes").toString("base64"),
    }],
  }
  const result = spawnSync(process.execPath, [broker, "--stdio"], {
    input: `${JSON.stringify(request)}\n`,
    encoding: "utf8",
    env: {
      ...process.env,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_BROKER_INPUT_ROOT: inputRoot,
      CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })
  const response = JSON.parse(result.stdout)
  const brokerStderr = Buffer.from(response.stderrBase64, "base64").toString()
  assert.equal(result.status, 0, `${result.stderr}${brokerStderr}`)
  assert.equal(response.status, 0, brokerStderr)
  assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), "credential-bytes")
  assert.deepEqual(await access(inputRoot).then(() => true, () => false), true)
  assert.deepEqual(await import("node:fs/promises").then(({ readdir }) => readdir(inputRoot)), [])

  const injected = validate({
    ...request,
    environment: { ...request.environment, CHARIOX_SLICE_CODEX_AUTH: "/etc/passwd" },
    files: [],
  }, share)
  assert.equal(injected.status, 1)
})

test("managed slice broker pins a provisioner path inode across caller replacement", async (context) => {
  if (process.platform !== "linux" || process.env.CHARIOX_RUN_PRIVILEGED_MOUNT_TESTS !== "1") return
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-pin-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  const workspace = join(share, "workspace")
  const moved = join(share, "workspace-original")
  const outside = join(root, "outside")
  const started = join(root, "started")
  const release = join(root, "release")
  await mkdir(workspace, { recursive: true })
  await mkdir(outside)
  await writeFile(join(workspace, "value"), "safe")
  await writeFile(join(outside, "value"), "secret")
  const provisioner = join(root, "provisioner.sh")
  await writeFile(provisioner, `#!/bin/sh
set -eu
: > '${started}'
while [ ! -e '${release}' ]; do sleep 0.01; done
cat "$CHARIOX_SLICE_WORKSPACE/value"
`)
  await chmod(provisioner, 0o755)
  const child = spawn(process.execPath, [broker, "--stdio"], {
    stdio: ["pipe", "pipe", "pipe"],
    env: {
      ...process.env,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })
  context.after(() => child.kill())
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
      CHARIOX_SLICE_WORKSPACE: workspace,
    },
    files: [],
  })}\n`)
  await waitFor(() => access(started).then(() => true, () => false))
  await rename(workspace, moved)
  await symlink(outside, workspace)
  await writeFile(release, "go")
  await waitFor(() => Promise.resolve(stdout.includes("\n")))
  child.stdin.end()
  await once(child, "exit")
  assert.equal(stderr, "")
  const response = JSON.parse(stdout)
  assert.equal(response.status, 0, Buffer.from(response.stderrBase64, "base64").toString())
  assert.equal(Buffer.from(response.stdoutBase64, "base64").toString(), "safe")
})

test("managed slice broker recovers after an oversized command output", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-output-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  await mkdir(share)
  const provisioner = join(root, "provisioner.sh")
  await writeFile(provisioner, "#!/bin/sh\nif [ \"$1\" = provision ]; then head -c 5242880 /dev/zero; else printf valid; fi\n")
  await chmod(provisioner, 0o755)
  const base = {
    kind: "provisioner",
    environment: {
      CHARIOX_SLICE_NAME: "chariox-slice-dev",
      CHARIOX_SLICE_ID: "slice-dev",
      CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-dev-home",
    },
    files: [],
  }
  const result = spawnSync(process.execPath, [broker, "--stdio"], {
    input: `${JSON.stringify({ ...base, action: "provision" })}\n${JSON.stringify({ ...base, action: "stop" })}\n`,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    env: {
      ...process.env,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
    },
  })
  assert.equal(result.status, 0, result.stderr)
  const responses = result.stdout.trim().split("\n").map(JSON.parse)
  assert.equal(responses.length, 2)
  assert.equal(responses[0].status, 125)
  assert.equal(responses[1].status, 0)
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
