// MP-08 / MP-09 / MP-10 / MP-11: run inside every kernel runtime image.
import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { once } from "node:events"

const child = spawn("/bin/sh", ["-c", "exec sleep 30"], { stdio: "ignore" })
const exited = once(child, "exit")
try {
  await once(child, "spawn")
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, "probe requires an owned child PID")
  const sample = spawnSync("ps", ["-axo", "pid=,ppid=,rss=,pcpu=,etime=,comm="], { encoding: "utf8" })
  if (sample.error) throw sample.error
  assert.equal(sample.status, 0, "the runtime must support every process sampler column")
  const row = sample.stdout.split("\n").map(line => line.trim().split(/\s+/)).find(fields => Number(fields[0]) === child.pid)
  assert.ok(row && row.length >= 6, "the launched process must appear in the image's process table")
  assert.equal(Number(row[1]), process.pid, "the sampler must identify the launched process's parent")
  assert.ok(Number.isFinite(Number(row[2])) && Number.isFinite(Number(row[3])))
  assert.match(row[4], /^(?:\d+-)?\d+(?::\d+){1,2}$/)
  console.log("process sampler image launch check passed")
} finally {
  if (Number.isSafeInteger(child.pid) && child.pid > 1 && child.exitCode === null && child.signalCode === null) child.kill("SIGTERM")
  await exited
}
