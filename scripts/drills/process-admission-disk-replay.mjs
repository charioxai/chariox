// MP-08 / MP-09 / MP-10 / MP-11: an exited writer's remembered output volume
// must let the same command run again while that filesystem has room.
import assert from "node:assert/strict"
import { existsSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { join } from "node:path"

export async function runDiskReplay(ctx, check) {
  const { agents, ask, work, dir, until, note, report } = ctx
  assert.equal(agents.length, 1, "disk-replay needs one real provider")
  const output = join(work, "replay-output")
  const done = join(work, "replay-done.json")
  const peaks = join(dir, "chariox/state/admission-peaks.json")
  const bytes = 640 * 1024 ** 2
  writeFileSync(join(work, "disk-replay.py"), `import json, os, pathlib, sys, time
with open("replay-output", "wb") as out:
    device = os.fstat(out.fileno()).st_dev
    chunk = b"x" * (4 << 20)
    for _ in range(160):
        out.write(chunk)
        out.flush()
        os.fsync(out.fileno())
        time.sleep(0.1)
pathlib.Path("replay-done.json").write_text(json.dumps({"pid": os.getpid(), "argv": sys.argv, "bytes": 640 << 20, "output_device": device}))
`)
  const quote = text => "'" + text.replaceAll("'", "'\\''") + "'"
  const command = `cd ${quote(work)} && exec python3 disk-replay.py`
  const submit = () => ask(`Run this exact command once with a Bash tool timeout of 180000 ms: ${command}\nDo not change it, retry it, signal it or start it elsewhere if it waits.`, agents[0])
  const writerRows = () => ctx.latestRows().filter(row => row.owner?.id === agents[0].id && (row.state?.command_line ?? row.command)?.includes("disk-replay.py"))
  const exited = pid => {
    try { return ["Z", "X"].includes(readFileSync(`/proc/${pid}/stat`, "utf8").split(") ")[1].split(" ")[0]) }
    catch { return true }
  }
  report.replay_command = command
  try {
    await submit()
    await until(() => existsSync(done), "the first sampled writer to close its output", 120000)
    const first = JSON.parse(readFileSync(done, "utf8"))
    assert.equal(statSync(output).size, bytes)
    await until(() => exited(first.pid) && writerRows().length === 0, "the first writer to exit and leave the process feed", 30000)
    const remembered = await until(() => {
      if (!existsSync(peaks)) return false
      const entries = Object.entries(JSON.parse(readFileSync(peaks, "utf8"))).filter(([, peak]) => peak.volume_writes >= 512 * 1024 ** 2)
      return entries.length && entries
    }, "the real writer's heavy output sample to be remembered", 30000)
    report.replay_first = first
    report.replay_remembered = remembered
    note("replay-first-exited", { pid: first.pid, bytes, remembered })
    rmSync(output)
    rmSync(done)
    const stable = remembered.every(([, peak]) => {
      const volume = peak.volume
      return volume && typeof volume === "object" && volume.id === first.output_device && typeof volume.path === "string"
        && !volume.path.startsWith("/proc/") && existsSync(volume.path) && statSync(volume.path).dev === volume.id
    })
    check("the sampled receiving volume survives closing and removing the first output", stable)
    let diskWait = null
    await submit()
    await until(() => {
      diskWait = writerRows().find(row => ["held", "paused"].includes(row.state?.kind) && row.state.reason === "disk") ?? diskWait
      return existsSync(done) || diskWait
    }, "the same command to finish again on its healthy receiving volume", 120000)
    const finished = existsSync(done)
    report.replay_disk_wait = diskWait
    check("the exact same command reruns without waiting for disk on the healthy volume", finished && !diskWait, { disk_wait: diskWait })
    if (finished) {
      const second = JSON.parse(readFileSync(done, "utf8"))
      report.replay_second = second
      check("the second writer recreated its output on the same device", statSync(output).size === bytes && second.output_device === first.output_device && JSON.stringify(second.argv) === JSON.stringify(first.argv))
      await until(() => exited(second.pid) && writerRows().length === 0, "the second writer to exit", 30000)
    }
    assert(stable && finished && !diskWait, "the exited-writer receiving-volume regression failed")
  } finally {
    rmSync(output, { force: true })
    rmSync(done, { force: true })
  }
}
