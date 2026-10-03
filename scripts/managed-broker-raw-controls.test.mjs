import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { access, chmod, mkdir, mkdtemp, readFile, readdir, readlink, rm, writeFile } from "node:fs/promises"
import { createConnection } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const broker = fileURLToPath(new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs", import.meta.url))

async function waitFor(check, timeoutMs = 3000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await check()) return
    await new Promise(resolve => setTimeout(resolve, 20))
  }
  throw new Error("broker fixture did not settle")
}

async function namespaceProcesses(namespace) {
  const matches = await Promise.all((await readdir("/proc")).filter(pid => /^\d+$/.test(pid)).map(async pid => {
    if (await readlink(`/proc/${pid}/ns/mnt`).catch(() => null) !== namespace) return null
    return readFile(`/proc/${pid}/cmdline`, "utf8").catch(() => "")
  }))
  return matches.filter(value => value !== null)
}

async function fixture(context, dockerBody) {
  if (process.platform !== "linux" || process.env.CHARIOX_RUN_PRIVILEGED_MOUNT_TESTS !== "1") {
    context.skip("requires explicitly enabled Linux mount/PID namespaces")
    return
  }
  const root = await mkdtemp(join(tmpdir(), "chariox-reviewfix-broker-"))
  const share = join(root, "share"), output = join(root, "output"), artifacts = join(root, "artifacts")
  const socketPath = join(root, "lease.sock"), docker = join(root, "docker.py")
  let child, lease, namespace
  context.after(async () => {
    lease?.destroy()
    if (child && child.exitCode === null && child.signalCode === null) {
      const closed = once(child, "close")
      child.kill("SIGKILL") // unshare also kills the private PID namespace init.
      await closed
    }
    if (namespace) await waitFor(async () => (await namespaceProcesses(namespace)).length === 0)
    await rm(root, { recursive: true, force: true })
    assert.equal(await access(root).then(() => true, () => false), false)
  })
  for (const directory of [share, output, artifacts]) await mkdir(directory, { mode: 0o700 })
  await writeFile(docker, `#!/usr/bin/python3\nimport os,sys,signal,time,pathlib\n${dockerBody.replaceAll("__FIXTURE_ROOT__", JSON.stringify(root))}\n`)
  await chmod(docker, 0o755)
  const entry = join(root, "namespace.sh")
  await writeFile(entry, '#!/bin/sh\nset -eu\nmount --bind "$1" /usr/bin/docker\nexec "$2" "$3"\n')
  await chmod(entry, 0o755)
  child = spawn("/usr/bin/unshare", ["--mount", "--propagation", "private", "--pid", "--fork", "--kill-child=SIGKILL", "--mount-proc", entry, docker, process.execPath, broker], {
    detached: true, stdio: ["ignore", "pipe", "pipe"],
    env: { PATH: process.env.PATH, HOME: root,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share,
      CHARIOX_SLICE_DOCKER_BROKER_SOCKET: socketPath,
      CHARIOX_SLICE_DOCKER_BROKER_INPUT_ROOT: join(root, "input"),
      CHARIOX_SLICE_DOCKER_BROKER_OUTPUT_ROOT: output,
      CHARIOX_SLICE_DOCKER_BROKER_ARTIFACT_ROOT: artifacts,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"),
      CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
    },
  })
  let consoleOutput = ""
  child.stdout.on("data", chunk => { consoleOutput += chunk })
  child.stderr.on("data", chunk => { consoleOutput += chunk })
  await waitFor(() => access(socketPath).then(() => true, () => false))
  namespace = await readlink(`/proc/${child.pid}/ns/mnt`)
  assert.notEqual(namespace, await readlink("/proc/self/ns/mnt"))
  lease = createConnection(socketPath)
  await once(lease, "connect")
  let buffered = Buffer.alloc(0), pending
  lease.on("data", chunk => {
    buffered = Buffer.concat([buffered, chunk])
    if (buffered.length < 4 || buffered.length < 4 + buffered.readUInt32BE(0)) return
    const length = buffered.readUInt32BE(0)
    const response = JSON.parse(buffered.subarray(4, length + 4))
    buffered = buffered.subarray(length + 4)
    pending?.(response)
  })
  async function request(args, timeoutMs = 35000) {
    const payload = Buffer.from(JSON.stringify({ kind: "docker", args }))
    const header = Buffer.alloc(4); header.writeUInt32BE(payload.length)
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending = undefined; reject(new Error("raw control did not settle with its lease open")) }, timeoutMs)
      pending = response => { clearTimeout(timer); pending = undefined; resolve(response) }
      lease.write(Buffer.concat([header, payload]))
    })
  }
  return { root, output, request, namespace, lease, consoleOutput: () => consoleOutput }
}

for (const operation of ["info", "stop"]) test(`MP-08 MP-10 MP-11 stalled raw ${operation} settles with an open lease and permits the next request`, async context => {
  const instance = await fixture(context, `
if sys.argv[1]=='${operation}':
 signal.signal(signal.SIGTERM,signal.SIG_IGN)
 if os.fork()==0: time.sleep(1000)
 else:
  (pathlib.Path(__FIXTURE_ROOT__)/'started').touch()
  time.sleep(1000)
else: print('next-control',flush=True)`)
  if (!instance) return
  const args = operation === "info" ? ["info"] : ["stop", "chariox-slice-fixture"]
  const started = Date.now()
  const response = await instance.request(args)
  assert.equal(response.status, 124)
  assert.match(Buffer.from(response.stderrBase64, "base64").toString(), /slice command timed out/)
  assert(Date.now() - started < 35000)
  assert.equal(await access(join(instance.root, "started")).then(() => true, () => false), true)
  const survivors = await namespaceProcesses(instance.namespace)
  assert(!survivors.some(command => command.includes("/usr/bin/docker") || command.includes("slice-command-guard.py")), "owned command group survived settlement")
  assert.equal(instance.lease.destroyed, false)
  const next = await instance.request(["ps", "--format", "{{.Names}}"], 3000)
  assert.equal(next.status, 0)
  assert.equal(Buffer.from(next.stdoutBase64, "base64").toString().trim(), "next-control")
})

test("MP-08 MP-10 MP-11 raw auth probes discard both streams before logs or broker diagnostics", async context => {
  const sentinel = "synthetic-auth-output-must-be-discarded"
  const instance = await fixture(context, `
if sys.argv[1]=='exec':
 os.write(1,b'${sentinel}'*10000)
 os.write(2,b'${sentinel}'*10000)
 sys.exit(7 if 'failure' in sys.argv[4] else 0)
else:
 os.write(1,b'ordinary-diagnostics'*300000)
 print('ordinary-error-detail',file=sys.stderr)`)
  if (!instance) return
  for (const container of ["chariox-slice-success", "chariox-slice-failure"]) {
    for (const command of [["gh", "auth", "token", "--hostname", "github.com"], ["test", "-s", "/home/slice/.chariox/daemon/provider-accounts/account/codex/profile/codex/auth.json"]]) {
      const response = await instance.request(["exec", "-u", "slice", container, ...command], 3000)
      assert.equal(response.status, container.endsWith("failure") ? 7 : 0)
      assert(response.stdoutBase64 === "", "status probes must not return stdout")
      assert(response.stderrBase64 === "", "status probes must not return stderr")
    }
  }
  assert.equal(await access(join(instance.output, "logs")).then(() => true, () => false), false, "auth probes must not create retained logs")
  assert(!instance.consoleOutput().includes(sentinel))
  const ordinary = await instance.request(["info"], 3000)
  assert.equal(ordinary.status, 0)
  const directories = await readdir(join(instance.output, "logs"))
  assert.equal(directories.length, 1)
  const logs = join(instance.output, "logs", directories[0])
  assert.equal((await readFile(join(logs, "stdout.log"))).length, "ordinary-diagnostics".length * 300000)
  assert.equal((await readFile(join(logs, "stderr.log"), "utf8")).trim(), "ordinary-error-detail")
  assert.match(Buffer.from(ordinary.stderrBase64, "base64").toString(), /complete diagnostics/)
})
