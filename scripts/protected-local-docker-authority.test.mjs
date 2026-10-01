import test from "node:test"
import assert from "node:assert/strict"
import { validateLocalDevEnrollment, verifyLocalRootfulEngine } from "../apps/kernel/slice-linux-docker/protected-local-docker-authority.mjs"

const hash = "a".repeat(64)
const fresh = () => ({version: 1, topology: "linux-local-rootful-dev", ownerUid: 1000, ownerGid: 1000,
  engineId: "engine-test", socket: {path: "/run/docker.sock", dev: 1, ino: 2, uid: 0, gid: 107, mode: 0o660},
  helperImageId: `sha256:${hash}`, workerImageId: `sha256:${hash}`, workerKernelHash: hash,
  sourceDigest: `sha256:${hash}`, sourceRoot: `/usr/lib/chariox/slice-local-dev/${hash}`,
  controlRoot: "/var/lib/chariox/slice-local-dev/u-1000/private/layout"})
const info = () => ({ID: "engine-test", OSType: "linux", SecurityOptions: ["name=seccomp,profile=builtin"]})
const socket = () => ({isSocket: true, dev: 1, ino: 2, uid: 0, gid: 107, mode: 0o660})
const map = "         0          0 4294967295\n"

test("explicit local DEV enrollment binds one owner and exact installed source/storage paths", () => {
  assert.equal(validateLocalDevEnrollment(fresh(), 1000).topology, "linux-local-rootful-dev")
  for (const mutate of [r => r.topology = "managed", r => r.ownerUid = 1001,
    r => r.controlRoot += "/../layout", r => r.sourceRoot = "/tmp/source", r => r.socket.path = "tcp://localhost:2375",
    r => r.socket.uid = 1000, r => r.socket.mode = 0o666, r => r.helperImageId = "latest", r => r.extra = true]) {
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
    HostConfig: {Privileged: false, ReadonlyRootfs: true, NetworkMode: "none", PidMode: "", UsernsMode: "", IpcMode: "private",
      CapDrop: ["ALL"], CapAdd: ["CHOWN", "DAC_OVERRIDE", "FOWNER"], SecurityOpt: ["no-new-privileges"]},
    Mounts: [
      {Type: "bind", Source: "/run/docker.sock", Destination: "/run/docker.sock", RW: true},
      {Type: "bind", Source: record.sourceRoot, Destination: record.sourceRoot, RW: false},
      {Type: "bind", Source: "/etc/chariox/slice-local-dev/1000.json", Destination: "/etc/chariox/slice-local-dev/1000.json", RW: false},
      {Type: "bind", Source: record.controlRoot.slice(0, -7), Destination: record.controlRoot.slice(0, -7), RW: true},
      {Type: "bind", Source: "/tmp/chariox-local-broker-1000-aB12", Destination: "/tmp/chariox-local-broker-1000-aB12", RW: true},
    ]})
  assert.equal(verifyLocalHelperTopology(record, helper()), true)
  for (const mutate of [h => h.Image = `sha256:${"b".repeat(64)}`, h => h.HostConfig.Privileged = true,
    h => h.HostConfig.CapAdd.push("SYS_ADMIN"), h => h.HostConfig.NetworkMode = "host", h => h.HostConfig.PidMode = "host",
    h => h.HostConfig.ReadonlyRootfs = false, h => h.Mounts[1].RW = true, h => h.Mounts[3].Source = "/",
    h => h.Mounts.push({Type: "bind", Source: "/root", Destination: "/root", RW: true})]) {
    const actual = helper(); mutate(actual)
    assert.throws(() => verifyLocalHelperTopology(record, actual), /authority is unavailable/)
  }
})

test("a root-owned recreated socket binds current launch inode without changing durable engine policy", () => {
  const record = fresh(), current = {...socket(), ino: 99}
  assert.equal(verifyLocalRootfulEngine(record, current, info(), map, map, {dev: 1, ino: 99}).dataUid, 1001)
  assert.throws(() => verifyLocalRootfulEngine(record, current, info(), map, map, {dev: 1, ino: 2}))
  assert.throws(() => verifyLocalRootfulEngine(record, {...current, uid: 1000}, info(), map, map, {dev: 1, ino: 99}))
})
