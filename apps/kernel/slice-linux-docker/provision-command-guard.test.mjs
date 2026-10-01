import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { chmod, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

const directory = fileURLToPath(new URL(".", import.meta.url))
const provisioner = join(directory, "provision-linux-docker-slice.sh")

function alive(pid) {
  try { process.kill(pid, 0); return true } catch { return false }
}

async function runShell(root, body, { input = "", deadline = 4000, environment = {}, signalAfter } = {}) {
  const source = await readFile(provisioner, "utf8")
  const script = join(root, "invoke.sh")
  await writeFile(script, source.replace(/^SCRIPT_DIR=.*$/m, `SCRIPT_DIR='${directory}'`).replace('main "$@"', body))
  const child = spawn("bash", [script], {
    detached: true,
    env: { PATH: process.env.PATH, TMPDIR: root, HOME: root,
      CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: `sha256:${"b".repeat(64)}`, ...environment },
    stdio: ["pipe", "pipe", "pipe"],
  })
  let stdout = "", stderr = "", expired = false, closed = false
  child.stdout.on("data", chunk => { stdout += chunk })
  child.stderr.on("data", chunk => { stderr += chunk })
  child.stdin.end(input)
  const started = Date.now()
  const interrupt = signalAfter === undefined ? undefined : setTimeout(() => child.kill("SIGTERM"), signalAfter)
  const timer = setTimeout(() => {
    expired = true
    try { process.kill(-child.pid, "SIGKILL") } catch {}
  }, deadline)
  try {
    const status = await new Promise((resolve, reject) => {
      child.once("error", reject)
      child.once("close", (status) => { closed = true; resolve(status) })
    })
    return { status, stdout, stderr, expired, elapsed: Date.now() - started }
  } finally {
    clearTimeout(timer)
    clearTimeout(interrupt)
    if (!closed) { try { process.kill(-child.pid, "SIGKILL") } catch {} }
  }
}

test("the actual timeout wrapper settles a resistant command and its pipe-holding descendant", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-command-owner-"))
  const producer = join(root, "producer.py")
  const pids = join(root, "pids")
  try {
    await writeFile(producer, `import os, signal, subprocess, sys, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
p = subprocess.Popen([sys.executable, "-c", "import signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(30)"])
open(sys.argv[1], "w").write(str(os.getpid())+" "+str(p.pid))
print("started", flush=True)
time.sleep(30)
`)
    const result = await runShell(root, `run_with_timeout 1 python3 '${producer}' '${pids}'`)
    assert.equal(result.expired, false, "owned timeout must close descendant-held output before outer fixture deadline")
    assert.equal(result.status, 124, result.stderr)
    assert.ok(result.elapsed < 3000, result.elapsed)
    for (const pid of (await readFile(pids, "utf8")).split(" ").map(Number)) {
      assert.equal(alive(pid), false, `owned process ${pid} remains`)
    }
  } finally {
    // The outer test group owns every baseline child, including old-wrapper leaks.
    await rm(root, { recursive: true, force: true })
  }
})


test("the actual wrappers preserve binary inherited, file and HERE-string stdin", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-command-stdin-"))
  try {
    const input = join(root, "input")
    await writeFile(input, "file-input\n")
    for (const [body, supplied, expected] of [
      ["run_with_timeout 1 cat", "inherited-input\n", "inherited-input\n"],
      [`run_with_file_stdin_timeout 1 '${input}' cat`, "", "file-input\n"],
      ["run_with_timeout 1 cat <<< 'here-input'", "", "here-input\n"],
      ["run_with_timeout 1 python3 -c 'import sys;print(sys.stdin.buffer.read().hex(),end=chr(10))'", Buffer.from([0xff, 0, 65]), "ff0041\n"],
    ]) {
      const result = await runShell(root, body, { input: supplied })
      assert.equal(result.expired, false)
      assert.equal(result.status, 0, result.stderr)
      assert.equal(result.stdout, expected)
    }
  } finally { await rm(root, { recursive: true, force: true }) }
})

for (const mode of ["parent-exit", "closed-pipes", "cancel"]) test(`owned command settlement covers ${mode}`, async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-command-settle-"))
  try {
    const script = join(root, "producer.py"), pidPath = join(root, "pid")
    const target = mode === "parent-exit" ? "None" : "subprocess.DEVNULL"
    await writeFile(script, `import os,signal,subprocess,sys,time
p=subprocess.Popen([sys.executable,"-c","import signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(30)"],stdout=${target},stderr=${target})
open(sys.argv[1],"w").write(str(p.pid))
${mode === "parent-exit" || mode === "closed-pipes" ? "sys.exit(0)" : "time.sleep(30)"}
`)
    const result = await runShell(root, `run_with_timeout 1 python3 '${script}' '${pidPath}'`, mode === "cancel" ? { signalAfter: 300 } : {})
    assert.equal(result.expired, false, result.stderr)
    if (mode === "parent-exit") assert.equal(result.status, 124)
    if (mode === "closed-pipes") assert.equal(result.status, 0)
    const pid = Number(await readFile(pidPath, "utf8"))
    const until = Date.now() + 1000
    while (alive(pid) && Date.now() < until) await new Promise(resolve => setTimeout(resolve, 20))
    assert.equal(alive(pid), false, `owned descendant ${pid} remains after ${mode}`)
  } finally { await rm(root, { recursive: true, force: true }) }
})

test("build waiting is unchanged without the trusted broker marker", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-build-deadline-"))
  try {
    const body = `run_with_timeout() { printf 'bounded:%s\\n' "$*"; }
run_guarded_command() { printf 'owned:%s\\n' "$*"; }
run_build_command docker buildx build --load --tag fixture .`
    const ordinary = await runShell(root, body)
    const extension = await runShell(root, body, { environment: { CHARIOX_SLICE_MANAGED_DOCKER_HOST: "unix:///public-fixture" } })
    for (const result of [ordinary, extension]) {
      assert.equal(result.status, 0, result.stderr)
      assert.match(result.stdout, /^owned:unbounded -- docker buildx build/)
    }
    const broker = await runShell(root, body, { environment: { CHARIOX_SLICE_BROKER_BUILD_TIMEOUT_SECONDS: "1200" } })
    assert.equal(broker.status, 0, broker.stderr)
    assert.match(broker.stdout, /^bounded:1200 docker buildx build/)
    const invalid = await runShell(root, body, { environment: { CHARIOX_SLICE_BROKER_BUILD_TIMEOUT_SECONDS: "20" } })
    assert.equal(invalid.status, 1)
    assert.match(invalid.stderr, /invalid broker build deadline/)
  } finally { await rm(root, { recursive: true, force: true }) }
})


test("broken consumer output settles a resistant command before the anchor exits", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-command-output-"))
  let child, closed = false
  try {
    const producer = join(root, "producer.py"), pidPath = join(root, "pid")
    await writeFile(producer, `import os,signal,sys,time
signal.signal(signal.SIGTERM,signal.SIG_IGN)
open(sys.argv[1],"w").write(str(os.getpid()))
while True:
 print("public-fixture",flush=True)
 time.sleep(.01)
`)
    child = spawn("python3", [join(directory, "slice-command-guard.py"), "unbounded", "--", "python3", producer, pidPath], { stdio: ["ignore", "pipe", "pipe"] })
    let stderr = ""
    child.stderr.on("data", bytes => { stderr += bytes })
    const result = new Promise((resolve, reject) => {
      child.once("error", reject)
      child.once("close", status => { closed = true; resolve(status) })
    })
    const timer = setTimeout(() => child.kill("SIGTERM"), 4000)
    try {
      await new Promise(resolve => child.stdout.once("data", resolve))
      child.stdout.destroy()
      assert.equal(await result, 1, stderr)
      const pid = Number(await readFile(pidPath, "utf8"))
      const until = Date.now() + 1000
      while (alive(pid) && Date.now() < until) await new Promise(resolve => setTimeout(resolve, 20))
      assert.equal(alive(pid), false)
    } finally { clearTimeout(timer) }
  } finally {
    if (child && !closed) {
      child.kill("SIGTERM")
      await new Promise(resolve => child.once("close", resolve))
    }
    await rm(root, { recursive: true, force: true })
  }
})

test("both Buildx branches use the build-specific owner", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-build-branch-"))
  try {
    const body = `docker() { [[ "$*" == 'buildx version' ]]; }
run_build_command() { printf '%s\\n' "$*"; }
docker_build --tag public-fixture .`
    const ordinary = await runShell(root, body)
    assert.equal(ordinary.status, 0, ordinary.stderr)
    assert.equal(ordinary.stdout.trim(), "docker buildx build --load --tag public-fixture .")
    const extension = await runShell(root, body, { environment: { CHARIOX_SLICE_MANAGED_DOCKER_HOST: "unix:///public-fixture" } })
    assert.equal(extension.status, 0, extension.stderr)
    assert.equal(extension.stdout.trim(), "docker buildx build --builder default --load --tag public-fixture .")
  } finally { await rm(root, { recursive: true, force: true }) }
})

test("interactive Docker keeps its inherited TTY and exit status", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-command-pty-"))
  try {
    const docker = join(root, "docker"), shell = join(root, "invoke.sh")
    await writeFile(docker, '#!/usr/bin/env python3\nimport os,sys\nprint("tty="+str([os.isatty(i) for i in range(3)]),flush=True)\nsys.exit(7)\n')
    await chmod(docker, 0o755)
    const source = await readFile(provisioner, "utf8")
    await writeFile(shell, source.replace(/^SCRIPT_DIR=.*$/m, `SCRIPT_DIR='${directory}'`).replace('main "$@"', 'docker exec -it public-fixture bash'))
    const driver = `import errno,os,pty,subprocess,sys
m,s=pty.openpty()
p=subprocess.Popen(["bash",sys.argv[1]],stdin=s,stdout=s,stderr=s)
os.close(s)
data=b""
while True:
 try: chunk=os.read(m,4096)
 except OSError as e:
  if e.errno==errno.EIO: break
  raise
 if not chunk: break
 data+=chunk
os.close(m)
sys.stdout.buffer.write(data)
sys.exit(p.wait())
`
    const result = spawnSync("python3", ["-c", driver, shell], { encoding: "utf8", timeout: 4000, env: { PATH: `${root}:${process.env.PATH}`, HOME: root, TMPDIR: root, CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: `sha256:${"b".repeat(64)}` } })
    assert.equal(result.status, 7, result.stderr)
    assert.match(result.stdout, /tty=\[True, True, True\]/)
  } finally { await rm(root, { recursive: true, force: true }) }
})

test("saved-home hashing uses the exact packaged inactivity policy and digest", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-command-digest-"))
  try {
    const archive = join(root, "archive")
    const bytes = Buffer.alloc(1024 * 1024, 0xff)
    await writeFile(archive, bytes)
    const { createHash } = await import("node:crypto")
    const result = await runShell(root, `SLICE_SAVED_HOME_ARCHIVE='${archive}'; saved_home_archive_identity`)
    assert.equal(result.status, 0, result.stderr)
    assert.equal(result.stdout.trim(), createHash("sha256").update(bytes).digest("hex"))
    const probe = spawnSync("python3", ["-c", `import importlib.util,sys
s=importlib.util.spec_from_file_location("guard",sys.argv[1]);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);print(m.progress_timeout_seconds())`, join(directory, "slice-command-guard.py")], { encoding: "utf8", env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" } })
    assert.equal(probe.status, 0, probe.stderr)
    assert.equal(probe.stdout.trim(), "300.0")
  } finally { await rm(root, { recursive: true, force: true }) }
})


test("native runtime hashing preserves the existing listed path projection", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-runtime-digest-"))
  try {
    const { createHash } = await import("node:crypto")
    const repository = join(root, "repository")
    await mkdir(join(repository, "apps/kernel"), { recursive: true })
    await mkdir(join(repository, "apps/relay"), { recursive: true })
    await writeFile(join(repository, "Cargo.toml"), "")
    await writeFile(join(repository, "apps/kernel/space file.txt"), Buffer.alloc(1024 * 1024, 0xff))
    await writeFile(join(repository, "apps/kernel/quoted\nname.txt"), "Git quotes this path; existing projection skips it")
    await writeFile(join(repository, "irrelevant.txt"), "outside the runtime membership")
    await symlink("../kernel/space file.txt", join(repository, "apps/relay/link"))
    for (const args of [["init", "--quiet"], ["add", "."]]) {
      const initialized = spawnSync("git", args, { cwd: repository, encoding: "utf8", env: { PATH: process.env.PATH, HOME: root } })
      assert.equal(initialized.status, 0, initialized.stderr)
    }
    const listed = spawnSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "Cargo.toml", "Cargo.lock", "adapters/rust", "apps/aegs-dummy", "apps/kernel", "apps/relay", "examples/workflow-code", "packages/aegs-sdk", "packages/event-protocol"], { cwd: repository, encoding: "utf8", env: { PATH: process.env.PATH, HOME: root } })
    assert.equal(listed.status, 0, listed.stderr)
    const expected = createHash("sha256")
    for (const path of listed.stdout.trimEnd().split("\n")) {
      const bytes = await readFile(join(repository, path)).catch(() => undefined)
      if (bytes) expected.update(`${path} ${createHash("sha256").update(bytes).digest("hex")}\n`)
    }
    const result = await runShell(root, `unset CHARIOX_SLICE_BUILD_CONTEXT_DIGEST; REPO_ROOT='${repository}'; runtime_source_revision`)
    assert.equal(result.status, 0, result.stderr)
    assert.equal(result.stdout.trim(), expected.digest("hex"))
    const empty = spawnSync("python3", [join(directory, "slice-command-guard.py"), "digest-paths", repository], { input: "", encoding: "utf8", timeout: 4000 })
    assert.equal(empty.status, 0, empty.stderr)
    assert.equal(empty.stdout.trim(), createHash("sha256").digest("hex"))
  } finally { await rm(root, { recursive: true, force: true }) }
})


test("control inspection failures are not successful absence decisions", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-control-failure-"))
  try {
    for (const body of [
      `docker() { return 124; }; stop_container`,
      `docker() { if [[ "$1" == ps && "$2" == -a ]]; then printf '%s\\n' "$SLICE_NAME"; else return 124; fi; }; stop_container`,
      `docker() { if [[ "$1" == ps ]]; then return 0; fi; printf 'daemon unavailable' >&2; return 1; }; destroy_container`,
      `docker() { if [[ "$1" == ps ]]; then return 0; fi; printf 'slice command timed out' >&2; return 124; }; destroy_container`,
    ]) {
      const result = await runShell(root, body)
      assert.equal(result.status, 1, result.stderr)
      assert.match(result.stderr, /failed to inspect/)
      assert.doesNotMatch(result.stderr, /removing volume/)
    }
    const missing = await runShell(root, `docker() { if [[ "$1" == ps ]]; then return 0; fi; printf 'Error: No such volume: public-fixture' >&2; return 1; }; destroy_container`)
    assert.equal(missing.status, 0, missing.stderr)
  } finally { await rm(root, { recursive: true, force: true }) }
})
