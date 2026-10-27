import test from "node:test"
import assert from "node:assert/strict"
import { validateLocalDevEnrollment, verifyLocalRootfulEngine } from "../apps/kernel/slice-linux-docker/protected-local-docker-authority.mjs"

const hash = "a".repeat(64)
const fresh = () => ({version: 1, topology: "linux-local-rootful-dev", providerSandboxCompatibility: true, ownerUid: 1000, ownerGid: 1000,
  engineId: "engine-test", socket: {path: "/run/docker.sock", dev: 1, ino: 2, uid: 0, gid: 107, mode: 0o660},
  helperImageId: `sha256:${hash}`, workerImageId: `sha256:${hash}`, workerKernelHash: hash, workerRuntimeRevision: "b".repeat(64),
  sourceDigest: `sha256:${hash}`, sourceRoot: `/usr/lib/chariox/slice-local-dev/${hash}`,
  controlRoot: "/var/lib/chariox/slice-local-dev/u-1000/private/layout"})
const info = () => ({ID: "engine-test", OSType: "linux", SecurityOptions: ["name=seccomp,profile=builtin"]})
const socket = () => ({isSocket: true, dev: 1, ino: 2, uid: 0, gid: 107, mode: 0o660})
const map = "         0          0 4294967295\n"

test("explicit local DEV enrollment binds one owner and exact installed source/storage paths", () => {
  assert.equal(validateLocalDevEnrollment(fresh(), 1000).topology, "linux-local-rootful-dev")
  for (const mutate of [r => r.topology = "managed", r => r.ownerUid = 1001,
    r => r.controlRoot += "/../layout", r => r.sourceRoot = "/tmp/source", r => r.socket.path = "tcp://localhost:2375",
    r => r.socket.uid = 1000, r => r.socket.mode = 0o666, r => r.helperImageId = "latest", r => r.workerRuntimeRevision = "latest", r => r.extra = true]) {
    const record = fresh(); mutate(record)
    assert.throws(() => validateLocalDevEnrollment(record, 1000), /authority is unavailable/)
  }
})

test("engine authority binds socket inode, daemon ID and full rootful UID/GID maps", () => {
  const record = validateLocalDevEnrollment(fresh(), 1000)
  assert.equal(verifyLocalRootfulEngine(record, socket(), info(), map, map).dataUid, 1001)
  for (const mutate of [s => s.dev++, s => s.ino++, s => s.uid++, s => s.gid++, s => s.mode = 0o600, s => s.isSocket = false]) {
    const value = socket(); mutate(value)
    assert.throws(() => verifyLocalRootfulEngine(record, value, info(), map, map), /authority is unavailable/)
  }
  for (const value of [{...info(), ID: "foreign"}, {...info(), OSType: "windows"},
    {...info(), SecurityOptions: ["name=rootless"]}, {...info(), SecurityOptions: ["name=userns"]}]) {
    assert.throws(() => verifyLocalRootfulEngine(record, socket(), value, map, map), /authority is unavailable/)
  }
  assert.throws(() => verifyLocalRootfulEngine(record, socket(), info(), "0 1000 1\n1 231072 65536", map))
  assert.throws(() => verifyLocalRootfulEngine(record, socket(), info(), map, "0 1000 1\n1 231072 65536"))
})

test("helper topology refuses privileged, foreign image and extra/control mount exposure", async () => {
  const {verifyLocalHelperTopology} = await import("../apps/kernel/slice-linux-docker/protected-local-docker-authority.mjs")
  const record = fresh()
  const helper = () => ({Image: record.helperImageId, Config: {User: "0:0"},
    HostConfig: {Privileged: false, ReadonlyRootfs: true, Init: true, NetworkMode: "none", PidMode: "", UsernsMode: "", IpcMode: "private",
      Tmpfs: {"/tmp":"rw,nosuid,nodev,size=128m", "/run/chariox-slice-broker":"rw,nosuid,nodev,size=32m"},
      CapDrop: ["ALL"], CapAdd: ["CHOWN", "DAC_OVERRIDE", "FOWNER"], SecurityOpt: ["no-new-privileges"]},
    Mounts: [
      {Type: "bind", Source: "/run/docker.sock", Destination: "/run/docker.sock", RW: true},
      {Type: "bind", Source: record.sourceRoot, Destination: record.sourceRoot, RW: false},
      {Type: "bind", Source: "/etc/chariox/slice-local-dev/1000.json", Destination: "/etc/chariox/slice-local-dev/1000.json", RW: false},
      {Type: "bind", Source: record.controlRoot.slice(0, -7), Destination: record.controlRoot.slice(0, -7), RW: true},
      {Type: "bind", Source: "/tmp/chariox-local-broker-1000-aB12", Destination: "/tmp/chariox-local-broker-1000-aB12", RW: true},
    ]})
  assert.equal(verifyLocalHelperTopology(record, helper()), true)
  // Current engines (Docker 29) report the canonical CAP_ form.
  const canonical = helper()
  canonical.HostConfig.CapAdd = ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FOWNER"]
  assert.equal(verifyLocalHelperTopology(record, canonical), true)
  for (const mutate of [h => h.Image = `sha256:${"b".repeat(64)}`, h => h.HostConfig.Privileged = true,
    h => h.HostConfig.CapAdd.push("SYS_ADMIN"), h => h.HostConfig.CapAdd.push("CAP_SYS_ADMIN"),
    h => h.HostConfig.CapAdd = ["CAP_CHOWN", "CAP_DAC_OVERRIDE"], h => h.HostConfig.CapDrop = ["CAP_CHOWN"], h => h.HostConfig.NetworkMode = "host", h => h.HostConfig.PidMode = "host",
    h => h.HostConfig.ReadonlyRootfs = false, h => h.HostConfig.Tmpfs["/etc"] = "rw", h => delete h.HostConfig.Tmpfs["/tmp"], h => h.Mounts[1].RW = true, h => h.Mounts[3].Source = "/",
    h => h.Mounts.push({Type: "bind", Source: "/root", Destination: "/root", RW: true})]) {
    const actual = helper(); mutate(actual)
    assert.throws(() => verifyLocalHelperTopology(record, actual), /authority is unavailable/)
  }
  // Without an init, Node is PID 1 and never reaps orphaned grandchildren.
  for (const mutate of [h => h.HostConfig.Init = false, h => delete h.HostConfig.Init]) {
    const actual = helper(); mutate(actual)
    assert.throws(() => verifyLocalHelperTopology(record, actual), /authority is unavailable/)
  }
})

test("the local helper runs Node under an init that reaps orphaned processes", async () => {
  const {readFileSync} = await import("node:fs")
  const launcher = readFileSync(new URL("../apps/kernel/slice-linux-docker/local-docker-broker-launch.mjs", import.meta.url), "utf8")
  assert.match(launcher, /\["run", "--rm", "--init", "--name", container,/)
})

test("a root-owned recreated socket binds current launch inode without changing durable engine policy", () => {
  const record = fresh(), current = {...socket(), ino: 99}
  assert.equal(verifyLocalRootfulEngine(record, current, info(), map, map, {dev: 1, ino: 99}).dataUid, 1001)
  assert.throws(() => verifyLocalRootfulEngine(record, current, info(), map, map, {dev: 1, ino: 2}))
  assert.throws(() => verifyLocalRootfulEngine(record, {...current, uid: 1000}, info(), map, map, {dev: 1, ino: 99}))
})

test("actual provisioner compatibility uses worker revision while first-boot provenance stays installed-source bound", async () => {
  const {localDevRuntimeEnvironment} = await import("../apps/kernel/slice-linux-docker/protected-local-docker-authority.mjs")
  const {readFileSync} = await import("node:fs")
  const {spawnSync} = await import("node:child_process")
  const source = readFileSync(new URL("../apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", import.meta.url),"utf8")
  const runtime = source.match(/^runtime_source_revision\(\) \{[\s\S]*?^\}/m)?.[0]
  const compatible = source.match(/^image_runtime_compatible\(\) \{[\s\S]*?^\}/m)?.[0]
  assert.ok(runtime && compatible)
  const enrollment = fresh(), env = localDevRuntimeEnvironment(enrollment)
  assert.equal(env.CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY, "1")
  assert.notEqual(env.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST, env.CHARIOX_SLICE_LOCAL_DEV_RUNTIME_REVISION)
  const run = overrides => spawnSync("/bin/bash", ["-ec", `${runtime}\n${compatible}\ndocker(){ case "$*" in *runtime-source-revision*) printf '%s' '${enrollment.workerRuntimeRevision}';; *relay-peer-protocol-version*) printf '42';; *) return 1;; esac; }\nSLICE_RELAY_PEER_PROTOCOL_VERSION=42\nSLICE_RUNTIME_SOURCE_REVISION=$(runtime_source_revision)\nimage_runtime_compatible '${enrollment.workerImageId}'\nprintf '%s' "$CHARIOX_SLICE_BUILD_CONTEXT_DIGEST"`],
    {encoding:"utf8",env:{PATH:"/usr/bin:/bin",...env,CHARIOX_SLICE_LOCAL_DEV_OWNER_UID:"1000",...overrides}})
  const passed=run({});assert.equal(passed.status,0,passed.stderr);assert.equal(passed.stdout,enrollment.sourceDigest)
  const refused=run({CHARIOX_SLICE_LOCAL_DEV_RUNTIME_REVISION:enrollment.sourceDigest});assert.notEqual(refused.status,0)
  assert.notEqual(run({CHARIOX_SLICE_LOCAL_DEV_RUNTIME_REVISION:""}).status,0)
  assert.throws(()=>localDevRuntimeEnvironment({...enrollment,workerRuntimeRevision:undefined}))
})

test("local DEV installer requires an explicit provider sandbox compatibility grant", async () => {
  const {spawnSync} = await import("node:child_process")
  const installer = new URL("../deploy/local-linux/install-local-docker-dev.py", import.meta.url)
  const result = spawnSync("python3", [installer.pathname], {encoding: "utf8"})
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /required:.*--allow-provider-sandbox-compatibility/)
})

test("local DEV compatibility requires a recorded boolean acknowledgement", async () => {
  const {localDevRuntimeEnvironment} = await import("../apps/kernel/slice-linux-docker/protected-local-docker-authority.mjs")
  const legacy = fresh()
  delete legacy.providerSandboxCompatibility
  assert.equal(localDevRuntimeEnvironment(legacy).CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY, "0")
  assert.equal(localDevRuntimeEnvironment({...fresh(), providerSandboxCompatibility: false}).CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY, "0")
  for (const value of ["true", 1, null, undefined]) {
    assert.throws(() => localDevRuntimeEnvironment({...fresh(), providerSandboxCompatibility: value}), /authority is unavailable/)
  }
})

test("the installer's serialized enrollment carries the acknowledgement into broker authority", async () => {
  const {spawnSync} = await import("node:child_process")
  const {localDevRuntimeEnvironment} = await import("../apps/kernel/slice-linux-docker/protected-local-docker-authority.mjs")
  // Evaluate only the real enrollment expression against synthetic public pins;
  // never execute the privileged installer or contact Docker.
  const result = spawnSync("python3", ["-c", `
import ast, json, pathlib, stat, sys
from types import SimpleNamespace
record = json.load(sys.stdin)
tree = ast.parse(pathlib.Path(sys.argv[1]).read_text())
expression = next(node.value for node in tree.body if isinstance(node, ast.Assign)
    and any(isinstance(target, ast.Name) and target.id == 'record' for target in node.targets))
a = SimpleNamespace(allow_provider_sandbox_compatibility=True, worker_image=record['workerImageId'],
    worker_kernel_sha256=record['workerKernelHash'], worker_runtime_revision=record['workerRuntimeRevision'])
user = SimpleNamespace(pw_uid=record['ownerUid'], pw_gid=record['ownerGid'])
socket = SimpleNamespace(**{'st_' + key: record['socket'][key] for key in ['dev', 'ino', 'uid', 'gid', 'mode']})
engine = {'ID': record['engineId']}
helper, digest, root, layout = record['helperImageId'], record['sourceDigest'][7:], record['sourceRoot'], record['controlRoot']
print(json.dumps(eval(compile(ast.Expression(expression), sys.argv[1], 'eval'))))
`, new URL("../deploy/local-linux/install-local-docker-dev.py", import.meta.url).pathname],
    {input: JSON.stringify(fresh()), encoding: "utf8"})
  assert.equal(result.status, 0, result.stderr)
  const enrollment = validateLocalDevEnrollment(JSON.parse(result.stdout), 1000)
  assert.equal(localDevRuntimeEnvironment(enrollment).CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY, "1")
})
