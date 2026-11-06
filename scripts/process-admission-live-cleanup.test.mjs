import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

// The drill needs root (a memory cgroup, a loop mount) and its state root under /root/.chariox/dev; elsewhere
// there is nothing to run, so the cases skip instead of pretending.
const drill = fileURLToPath(new URL("./drills/process-admission-live.mjs", import.meta.url))
const stateParent = "/root/.chariox/dev"
const runnable = process.getuid?.() === 0 && existsSync(stateParent) && existsSync("/sys/fs/cgroup/cgroup.controllers")
const have = tool => spawnSync("sh", ["-c", `command -v ${tool}`]).status === 0
const timeline = evidence => readFileSync(join(evidence, "timeline.jsonl"), "utf8").split("\n").filter(Boolean).map(line => JSON.parse(line))
const alive = pid => { try { process.kill(pid, 0); return !/^[ZX]/.test(readFileSync(`/proc/${pid}/stat`, "utf8").replace(/^.*\) /, "")) } catch { return false } }

// The kernel never answers: the readiness step is the injected setup failure.
const failSetup = (t, { diskGb }) => {
  const scratch = mkdtempSync(join(stateParent, "admission-cleanup-test-"))
  const evidence = mkdtempSync(join(tmpdir(), "admission-cleanup-evidence-"))
  const stateRoot = join(scratch, "state")
  const kernel = join(scratch, "kernel")
  writeFileSync(kernel, "#!/bin/sh\nexec sleep 600\n")
  chmodSync(kernel, 0o755)
  const workDir = join(stateParent, "kr06", `admission-cleanup-test-${process.pid}`)
  const args = ["--scenario", "memory", "--kernel-binary", kernel, "--state-root", stateRoot, "--evidence-dir", evidence,
    "--login-file", "/dev/null", "--ready-timeout-ms", "1500", "--settle-ms", "0"]
  if (diskGb) args.push("--disk-gb", "1", "--work-dir", workDir)
  const command = diskGb ? ["unshare", "-m", "--propagation", "private", process.execPath, drill, ...args] : [process.execPath, drill, ...args]
  const images = () => readdirSync(stateRoot).map(name => join(stateRoot, name, "volume.img"))
  t.after(() => {
    // Whatever the drill left behind is removed here, by exact pid, cgroup and loop device.
    if (existsSync(join(evidence, "timeline.jsonl"))) for (const row of timeline(evidence)) {
      if (row.name === "kernel-start") {
        if (row.pid > 1 && alive(row.pid)) process.kill(row.pid, "SIGKILL")
        spawnSync("sh", ["-c", `sleep 0.3; rmdir ${row.pool}/kernel ${row.pool}`])
      }
    }
    if (existsSync(stateRoot)) for (const image of images()) {
      for (const line of spawnSync("losetup", ["-j", image], { encoding: "utf8" }).stdout.split("\n").filter(Boolean)) spawnSync("losetup", ["-d", line.split(":")[0]])
    }
    rmSync(scratch, { recursive: true, force: true })
    rmSync(evidence, { recursive: true, force: true })
    rmSync(workDir, { recursive: true, force: true })
  })
  const run = spawnSync(command[0], command.slice(1), { encoding: "utf8", timeout: 120000 })
  return { run, evidence, images: existsSync(stateRoot) ? images() : [] }
}

const assertNothingLeft = ({ run, evidence, images }) => {
  assert.equal(run.status, 1, `the drill reports its failure: ${run.stderr}`)
  const start = timeline(evidence).find(row => row.name === "kernel-start")
  assert.ok(start, "the kernel was started")
  assert.ok(!existsSync(start.pool), "the pool cgroup is removed")
  assert.ok(!alive(start.pid), "the kernel process is stopped")
  for (const image of images) {
    assert.ok(!existsSync(image), "the loop image is removed")
    assert.equal(spawnSync("losetup", ["-j", image], { encoding: "utf8" }).stdout.trim(), "", "the loop device is detached")
  }
  const report = JSON.parse(readFileSync(join(evidence, "result.json"), "utf8"))
  assert.match(report.error, /kernel ready/, "the report names the failed step")
  assert.ok(existsSync(join(evidence, "os-trace.json")), "the evidence files are written")
}

test("a kernel that never becomes ready is cleaned up and reported", { skip: !runnable && "needs root, cgroup v2 and /root/.chariox/dev" }, t => {
  assertNothingLeft(failSetup(t, { diskGb: 0 }))
})

test("a failed setup detaches the loop volume and reports", { skip: (!runnable || !have("losetup") || !have("mkfs.ext4") || !have("unshare")) && "needs root, loop devices and unshare" }, t => {
  const result = failSetup(t, { diskGb: 1 })
  assert.equal(result.images.length, 1, "the volume was created")
  assertNothingLeft(result)
})
