import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { test } from "node:test"

const broker = fileURLToPath(new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs", import.meta.url))

async function fixture(context) {
  const share = await mkdtemp(join(tmpdir(), "chariox-broker-viewer-"))
  context.after(() => rm(share, { recursive: true, force: true }))
  return (request) => spawnSync(process.execPath, [broker, "--validate-request"], {
    input: JSON.stringify(request), encoding: "utf8", timeout: 5000,
    env: { ...process.env, CHARIOX_SLICE_DOCKER_SHARE_ROOT: share },
  })
}

function provision(action, backend) {
  return { kind: "provisioner", action, files: [], environment: {
    CHARIOX_SLICE_NAME: "chariox-slice-viewer",
    CHARIOX_SLICE_ID: "slice-viewer",
    CHARIOX_SLICE_HOME_VOLUME: "chariox-slice-viewer-home",
    ...(backend === undefined ? {} : { CHARIOX_SLICE_VIEWER_BACKEND: backend }),
  } }
}

// Exact argument order emitted by run_local_docker_slice_screen in local_docker.rs.
function screen(backend = "selkies", port = "6080", mode = "headed", action = "start") {
  return { kind: "docker", args: [
    "exec", "-e", `CHARIOX_SLICE_VIEWER_BACKEND=${backend}`,
    "-e", `CHARIOX_SLICE_NOVNC_PORT=${port}`,
    "-e", `CHARIOX_SLICE_DISPLAY_MODE=${mode}`,
    "-u", "slice", "chariox-slice-viewer", "/opt/chariox-slice/slice-screen.sh", action,
  ] }
}

const roomBinding = {
  CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID: "kernel-home",
  CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY: Buffer.concat([Buffer.from([4]), Buffer.alloc(64, 1)]).toString("base64"),
  CHARIOX_ROOM_ENVIRONMENT_SESSION_ID: "room-1",
  CHARIOX_ROOM_ENVIRONMENT_SLICE_ID: "slice-viewer",
}

test("managed broker accepts complete Room bindings on provision, restore and recover", async (context) => {
  const validate = await fixture(context)
  for (const action of ["provision", "restore-state", "recover"]) {
    const request = provision(action, "selkies")
    Object.assign(request.environment, roomBinding)
    const result = validate(request)
    assert.equal(result.status, 0, `${action}: ${result.stderr}`)
  }
})

test("managed broker execution delivers the Room binding to its provisioner child", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-broker-room-execution-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const provisioner = join(root, "provisioner")
  await writeFile(provisioner, `#!${process.execPath}
process.stdout.write(JSON.stringify(Object.fromEntries(Object.entries(process.env)
  .filter(([name]) => name.startsWith("CHARIOX_ROOM_ENVIRONMENT_")))))
`, { mode: 0o700 })
  for (const action of ["provision", "restore-state", "recover"]) {
    const request = provision(action, "selkies")
    Object.assign(request.environment, roomBinding)
    const result = spawnSync(process.execPath, [broker, "--stdio"], {
      input: JSON.stringify(request) + "\n", encoding: "utf8", timeout: 5000,
      env: { ...process.env,
        CHARIOX_SLICE_DOCKER_SHARE_ROOT: root,
        CHARIOX_SLICE_DOCKER_PROVISIONER: provisioner,
        CHARIOX_SLICE_DOCKER_BROKER_INPUT_ROOT: join(root, "input"),
        CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
        CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
      },
    })
    assert.equal(result.status, 0, result.stderr)
    const response = JSON.parse(result.stdout)
    assert.equal(response.status, 0, Buffer.from(response.stderrBase64, "base64").toString())
    assert.deepEqual(JSON.parse(Buffer.from(response.stdoutBase64, "base64").toString()), roomBinding)
  }
})

test("managed broker rejects incomplete or malformed Room bindings and unrelated actions", async (context) => {
  const validate = await fixture(context)
  for (const key of Object.keys(roomBinding)) {
    for (const value of [undefined, "", "invalid\n", null]) {
      const request = provision("provision", "selkies")
      Object.assign(request.environment, roomBinding, { [key]: value })
      assert.equal(validate(request).status, 1, `${key}/${value}`)
    }
  }
  for (const change of [
    { CHARIOX_ROOM_ENVIRONMENT_SLICE_ID: "slice-other" },
    { CHARIOX_ROOM_ENVIRONMENT_UNRECOGNIZED: "value" },
  ]) {
    const request = provision("provision", "selkies")
    Object.assign(request.environment, roomBinding, change)
    assert.equal(validate(request).status, 1)
  }
  for (const action of ["stop", "destroy", "import-provider-auth", "remove-provider-auth", "start-provider-login"]) {
    const request = provision(action)
    Object.assign(request.environment, roomBinding)
    assert.equal(validate(request).status, 1, action)
  }
})

test("managed broker permits the exact snapshot quiesce and image integrity commands", async (context) => {
  const validate = await fixture(context)
  for (const args of [
    ["pause", "chariox-slice-viewer"],
    ["unpause", "chariox-slice-viewer"],
    ["image", "inspect", "--format", "{{.Id}}", "chariox-slice-state:viewer-0123456789abcdef"],
    ["image", "inspect", "--format", "{{.Id}}", "chariox-slice-backup:viewer-0123456789abcdef"],
  ]) {
    const result = validate({ kind: "docker", args })
    assert.equal(result.status, 0, `${JSON.stringify(args)}: ${result.stderr}`)
  }
})

test("snapshot broker commands cannot target other resources or widen inspection", async (context) => {
  const validate = await fixture(context)
  for (const args of [
    ["pause", "other"], ["unpause", "chariox-relay"],
    ["pause", "chariox-slice-viewer", "chariox-slice-second"],
    ["unpause", "--all"],
    ["image", "inspect", "--format", "{{.Id}}", "ubuntu:latest"],
    ["image", "inspect", "--format", "{{json .}}", "chariox-slice-state:viewer"],
    ["image", "inspect", "chariox-slice-state:viewer"],
    ["image", "inspect", "--format", "{{.Id}}", "chariox-slice-state:viewer", "other"],
  ]) assert.equal(validate({ kind: "docker", args }).status, 1, JSON.stringify(args))
})

test("managed broker accepts the kernel viewer backend on provision, restore and recover", async (context) => {
  const validate = await fixture(context)
  for (const action of ["provision", "restore-state", "recover"]) {
    for (const backend of ["selkies", "novnc", undefined]) {
      const result = validate(provision(action, backend))
      assert.equal(result.status, 0, `${action}/${backend}: ${result.stderr}`)
    }
  }
})

test("managed broker rejects unknown viewer values and unrelated action overrides", async (context) => {
  const validate = await fixture(context)
  for (const backend of ["", "SELKIES", "other", "selkies\n", "selkies;sh", "novnc=1", null, 1]) {
    assert.equal(validate(provision("provision", backend)).status, 1, String(backend))
  }
  for (const action of ["stop", "destroy", "import-provider-auth", "remove-provider-auth", "start-provider-login"]) {
    assert.equal(validate(provision(action, "selkies")).status, 1, action)
  }
})

test("managed broker accepts kernel screen lifecycle commands with explicit viewer configuration", async (context) => {
  const validate = await fixture(context)
  for (const backend of ["selkies", "novnc"]) {
    for (const mode of ["headed", "headless"]) {
      for (const action of ["start", "stop", "status", "prepare", "interact"]) {
        const result = validate(screen(backend, "6080", mode, action))
        assert.equal(result.status, 0, `${backend}/${mode}/${action}: ${result.stderr}`)
      }
    }
  }
  const legacy = screen()
  legacy.args.splice(1, 6)
  assert.equal(validate(legacy).status, 0, "legacy screen calls remain supported")
})

test("managed broker confines screen overrides to known fields, values, user and script", async (context) => {
  const validate = await fixture(context)
  const rejected = [
    screen("other"), screen("selkies;sh"), screen("selkies", "-1"),
    screen("selkies", "65536"), screen("selkies", "1\n"), screen("selkies", ""),
    screen("selkies", "6080", "other"), screen("selkies", "6080", "headed", "exec"),
  ]
  for (const [index, value] of [[2, "LD_PRELOAD=/tmp/a"], [4, "CHARIOX_SLICE_VIEWER_BACKEND=novnc"],
    [8, "root"], [9, "chariox-other"], [10, "/bin/sh"]]) {
    const request = screen()
    request.args[index] = value
    rejected.push(request)
  }
  const extra = screen()
  extra.args.push("arbitrary")
  rejected.push(extra)
  const partial = screen()
  partial.args.splice(3, 2)
  rejected.push(partial)
  const injected = screen()
  injected.args.splice(7, 0, "-e", "PATH=/tmp")
  rejected.push(injected)
  for (const request of rejected) {
    assert.equal(validate(request).status, 1, JSON.stringify(request))
  }
})
