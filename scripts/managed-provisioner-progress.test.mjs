import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { once } from "node:events"
import { access, chmod, mkdir, mkdtemp, readFile, readdir, readlink, rm, stat, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const sourceRoot = process.env.CHARIOX_PROVISIONER_PROGRESS_SOURCE ?? repositoryRoot
const sliceDirectory = join(sourceRoot, "apps/kernel/slice-linux-docker")
const digest = `sha256:${"b".repeat(64)}`
const identity = { container: "chariox-slice-progress", volume: "chariox-slice-progress-home", slice: "slice-progress" }

async function waitFor(predicate, milliseconds = 3000) {
  const end = Date.now() + milliseconds
  while (Date.now() < end) {
    if (await predicate()) return
    await new Promise(resolve => setTimeout(resolve, 20))
  }
  throw new Error("owned production-path fixture did not settle before its outer deadline")
}

async function namespaceProcesses(namespace) {
  const entries = (await readdir("/proc")).filter(entry => /^\d+$/.test(entry))
  const matches = await Promise.all(entries.map(async pid => (
    await readlink(`/proc/${pid}/ns/mnt`).catch(() => undefined)
  ) === namespace ? pid : undefined))
  return matches.filter(Boolean)
}

const readStallSource = `
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
ssize_t read(int fd, void *buf, size_t count) {
  static ssize_t (*actual_read)(int, void *, size_t);
  if (!actual_read) actual_read = dlsym(RTLD_NEXT, "read");
  const char *dev = getenv("CHARIOX_TEST_ARCHIVE_DEVICE");
  const char *ino = getenv("CHARIOX_TEST_ARCHIVE_INODE");
  struct stat metadata;
  if (dev && ino && fstat(fd, &metadata) == 0 &&
      metadata.st_dev == strtoull(dev, NULL, 10) && metadata.st_ino == strtoull(ino, NULL, 10)) {
    int marker = open(getenv("CHARIOX_TEST_READ_MARKER"), O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (marker >= 0) { dprintf(marker, "%d\\n", getpid()); close(marker); }
    for (;;) { struct timespec pause = {1, 0}; nanosleep(&pause, NULL); }
  }
  return actual_read(fd, buf, count);
}
`

for (const fault of ["docker-stop", "archive-read"]) test(`actual archive-bearing provisioner settles ${fault} and the broker admits its next request`, async context => {
  if (process.platform !== "linux" || process.env.CHARIOX_RUN_PRIVILEGED_MOUNT_TESTS !== "1") {
    context.skip("requires an explicitly enabled Linux mount and PID namespace")
    return
  }
  const root = await mkdtemp(join(tmpdir(), "chariox-provisioner-progress-"))
  const share = join(root, "share")
  const artifactRoot = join(share, ".broker-private/artifacts")
  const archiveDirectory = join(artifactRoot, "states", identity.slice, "generation-abcdef")
  const archive = join(archiveDirectory, "home.tar.zst")
  const quota = join(root, "quota")
  const run = join(root, "run")
  const marker = join(root, "stalled")
  const operations = join(root, "docker-operations.jsonl")
  const fakeDocker = join(root, "docker")
  const shimDirectory = join(root, "bin")
  let child, namespace
  context.after(async () => {
    if (child && child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL")
      await once(child, "close")
    }
    if (namespace) await waitFor(async () => (await namespaceProcesses(namespace)).length === 0)
    const entries = await readdir(root)
    const allowed = new Set(["share", "quota", "run", "bin", "docker", "docker-operations.jsonl", "stops", "stalled", "release-manifest.json", "home-archive-policy.json", "stalled-read.c", "stalled-read.so", "reaper.py", "namespace.sh", "handles", "handles.json"])
    assert.ok(entries.every(entry => allowed.has(entry)), "cleanup inventory contains only this synthetic fixture's entries")
    await rm(root, { recursive: true, force: true })
  })
  for (const directory of [archiveDirectory, quota, run, shimDirectory]) await mkdir(directory, { recursive: true, mode: 0o700 })
  const prior = Buffer.from("synthetic saved home bytes; no provider profile")
  const metadata = JSON.stringify({ schemaVersion: 1, scope: "state", id: identity.slice,
    sizeBytes: prior.length, sha256: createHash("sha256").update(prior).digest("hex") })
  await writeFile(archive, prior, { mode: 0o600 })
  await writeFile(join(archiveDirectory, "metadata.json"), metadata, { mode: 0o600 })
  await writeFile(join(quota, "reservations.json"), JSON.stringify({ schemaVersion: 1,
    nextProjectId: 1073741824, reservations: {} }), { mode: 0o600 })
  const manifest = join(root, "release-manifest.json")
  await writeFile(manifest, JSON.stringify({ artifacts: [{ name: "chariox-slice-build-context",
    path: "/usr/lib/chariox/slice-build-context", sha256: digest }] }), { mode: 0o600 })
  await writeFile(fakeDocker, `#!${process.execPath}
const fs = require("node:fs"), {spawn} = require("node:child_process")
const args = process.argv.slice(2), text = args.join(" ")
fs.appendFileSync(${JSON.stringify(operations)}, JSON.stringify(args)+"\\n")
const fault = ${JSON.stringify(fault)}, container = ${JSON.stringify(identity.container)}, volume = ${JSON.stringify(identity.volume)}
if (text.includes("{{json .Config.Labels}}") || text.includes("{{json .Labels}}")) {
  process.stdout.write(JSON.stringify({"io.chariox.slice.id":${JSON.stringify(identity.slice)},"io.chariox.slice.owner-kernel-id":"kernel-progress","io.chariox.slice.owner-machine-id":"machine-progress"}))
} else if (text.includes("{{json .Mounts}}")) {
  if (fault === "archive-read") { console.error("Error: No such container: "+container); process.exit(1) }
  process.stdout.write("[]")
} else if (args[0] === "info") {
  process.stdout.write("amd64")
} else if (args[0] === "image" && args[1] === "inspect") {
  process.stdout.write(text.includes("relay-peer-protocol-version") ? "64" : text.includes("runtime-source-revision") ? ${JSON.stringify(digest)} : "synthetic-image")
} else if (args[0] === "ps") {
  if (fault === "docker-stop") process.stdout.write(container+"\\n")
} else if (args[0] === "stop") {
  const countFile = ${JSON.stringify(join(root, "stops"))}
  const count = Number(fs.existsSync(countFile) ? fs.readFileSync(countFile,"utf8") : 0)+1
  fs.writeFileSync(countFile,String(count))
  if (count === 2) {
    // Broker preflight succeeds. A concurrent restart is modeled by ps above,
    // so the actual tracked restore-state teardown reaches its own stop.
    process.on("SIGTERM",()=>{})
    const descendant=spawn("/bin/sleep",["1000"],{stdio:"inherit"})
    fs.writeFileSync(${JSON.stringify(marker)},JSON.stringify([process.pid,descendant.pid]))
    setInterval(()=>{},1000)
  }
} else if (args[0] === "container" && args[1] === "inspect") {
  console.error("Error: No such container: "+container); process.exit(1)
} else if (args[0] === "volume" && args[1] === "inspect") {
  console.error("Error: No such volume: "+volume); process.exit(1)
} else if (args[0] !== "exec") {
  console.error("unexpected fixture Docker operation: "+args[0]); process.exit(99)
}
`)
  await chmod(fakeDocker, 0o755)
  const policy = join(root, "home-archive-policy.json")
  const actualPolicy = JSON.parse(await readFile(join(sliceDirectory, "home-archive-policy.json"), "utf8"))
  await writeFile(policy, JSON.stringify({ ...actualPolicy, progressTimeoutMs: 100 }), { mode: 0o600 })
  if (fault === "archive-read") {
    const archiveStat = await stat(archive, { bigint: true })
    const injection = join(root, "stalled-read.so"), cSource = join(root, "stalled-read.c")
    await writeFile(cSource, readStallSource)
    const compiled = spawnSync("cc", ["-shared", "-fPIC", "-o", injection, cSource, "-ldl"], { encoding: "utf8" })
    assert.equal(compiled.status, 0, compiled.stderr)
    const environment = `LD_PRELOAD='${injection}' CHARIOX_TEST_ARCHIVE_DEVICE='${archiveStat.dev}' CHARIOX_TEST_ARCHIVE_INODE='${archiveStat.ino}' CHARIOX_TEST_READ_MARKER='${marker}'`
    await writeFile(join(shimDirectory, "python3"), `#!/bin/sh\nif [ "$2" = digest ]; then exec /usr/bin/env ${environment} /usr/bin/python3 "$@"; fi\nexec /usr/bin/python3 "$@"\n`)
    await writeFile(join(shimDirectory, "sha256sum"), `#!/bin/sh\nexec /usr/bin/env ${environment} /usr/bin/sha256sum "$@"\n`)
    await chmod(join(shimDirectory, "python3"), 0o755)
    await chmod(join(shimDirectory, "sha256sum"), 0o755)
  }
  const reaper = join(root, "reaper.py")
  await writeFile(reaper, `import os, subprocess, sys, time
child = subprocess.Popen(sys.argv[1:])
while True:
    try: pid, status = os.waitpid(-1, os.WNOHANG)
    except ChildProcessError: sys.exit(0)
    if pid == child.pid: sys.exit(os.waitstatus_to_exitcode(status))
    time.sleep(.01)
`)
  const entrypoint = join(root, "namespace.sh")
  await writeFile(entrypoint, `#!/bin/sh
set -eu
mount --bind "$1" /usr/bin/docker
mount --bind "$2" /var/lib/chariox-slice-disk-quota
mount --bind "$3" /run
if [ "$4" = archive-read ]; then
  mount --bind "$5" /usr/local/bin
  mount --bind "$6" "$7/home-archive-policy.json"
fi
exec /usr/bin/python3 "$8" "$9" "\${10}" --stdio
`)
  await chmod(entrypoint, 0o755)
  child = spawn("/usr/bin/unshare", ["--mount", "--propagation", "private", "--pid", "--fork", "--kill-child=SIGKILL", "--mount-proc", entrypoint,
    fakeDocker, quota, run, fault, shimDirectory, policy, sliceDirectory, reaper, process.execPath, join(sliceDirectory, "managed-docker-broker.mjs")], {
    stdio: ["pipe", "pipe", "pipe"], env: { PATH: "/usr/bin:/bin", HOME: root,
      CHARIOX_SLICE_DOCKER_SHARE_ROOT: share, CHARIOX_SLICE_DOCKER_BROKER_ARTIFACT_ROOT: artifactRoot,
      CHARIOX_SLICE_DOCKER_HANDLE_ROOT: join(root, "handles"), CHARIOX_SLICE_DOCKER_HANDLE_STATE: join(root, "handles.json"),
      CHARIOX_MANAGED_RELEASE_MANIFEST: manifest,
    },
  })
  let stdout = "", stderr = ""
  child.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk })
  child.stderr.setEncoding("utf8").on("data", chunk => { stderr += chunk })
  await waitFor(async () => await access(marker).then(() => true, () => false) || Boolean(stderr), 100).catch(() => {})
  namespace = await readlink(`/proc/${child.pid}/ns/mnt`)
  assert.notEqual(namespace, await readlink("/proc/self/ns/mnt"))
  const request = { kind: "provisioner", action: fault === "docker-stop" ? "restore-state" : "provision", files: [], environment: {
    CHARIOX_SLICE_NAME: identity.container, CHARIOX_SLICE_ID: identity.slice, CHARIOX_SLICE_HOME_VOLUME: identity.volume,
    CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-progress", CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-progress",
    CHARIOX_SLICE_SAVED_HOME_ARCHIVE: archive, CHARIOX_SLICE_BUILD_IMAGE: "never", CHARIOX_SLICE_VIEWER_BACKEND: "novnc",
    CHARIOX_SLICE_START_DESKTOP: "0", CHARIOX_SLICE_START_PROVIDER_SERVERS: "0",
  } }
  child.stdin.write(JSON.stringify(request)+"\n")
  await waitFor(async () => await access(marker).then(() => true, () => false) || stdout.includes("\n"))
  assert.equal(await access(marker).then(() => true, () => false), true, stdout || stderr)
  context.diagnostic(`${fault}: initial broker archive verification and tracked shell dispatch reached the injected fault`)
  await waitFor(() => stdout.includes("\n"), fault === "docker-stop" ? 35_000 : 3500)
  const first = JSON.parse(stdout.split("\n")[0])
  assert.notEqual(first.status, 0, "a stalled tracked production operation must fail")
  assert.match(Buffer.from(first.stderrBase64, "base64").toString(), fault === "docker-stop" ? /timed out/ : /progress deadline/)
  const stalledPids = fault === "docker-stop" ? JSON.parse(await readFile(marker, "utf8")) : [Number(await readFile(marker, "utf8"))]
  for (const pid of stalledPids) await waitFor(async () => access(`/proc/${child.pid}/root/proc/${pid}`).then(() => false, () => true))
  assert.deepEqual(await readFile(archive), prior, "failure preserves prior archive bytes")
  assert.equal(await readFile(join(archiveDirectory, "metadata.json"), "utf8"), metadata, "failure preserves prior generation metadata")
  assert.equal((await stat(archive)).mode & 0o777, 0o600)
  assert.equal((await stat(join(archiveDirectory, "metadata.json"))).mode & 0o777, 0o600)
  const calls = (await readFile(operations, "utf8")).trim().split("\n").map(JSON.parse)
  assert.equal(calls.some(args => args[0] === "volume" && ["rm", "create"].includes(args[1])), false, "failure occurs before any home-volume mutation")
  child.stdin.write(JSON.stringify({ kind: "docker", args: ["image", "inspect", "--format", "{{.Id}}", "registry.example/progress:verified"] })+"\n")
  await waitFor(() => stdout.trim().split("\n").length === 2)
  const next = JSON.parse(stdout.trim().split("\n")[1])
  assert.equal(next.status, 0, Buffer.from(next.stderrBase64, "base64").toString())
  assert.equal(Buffer.from(next.stdoutBase64, "base64").toString(), "synthetic-image")
  child.stdin.end()
  await once(child, "close")
  assert.equal(child.exitCode, 0, stderr)
})
