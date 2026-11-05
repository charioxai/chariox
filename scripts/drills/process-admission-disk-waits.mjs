// MP-08 / MP-09 / MP-10 / MP-11: two real providers wait on separate volumes;
// recovering only the later writer's volume must let it finish.
import assert from "node:assert/strict"
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { join } from "node:path"

const GIB = 1024 ** 3
export async function runDiskWaits(ctx, check) {
  const { agents, ask, work, dir, execFileSync, until, note, report } = ctx
  assert.equal(agents.length, 2)
  const other = join(dir, "other-volume"), image = join(dir, "other-volume.img")
  mkdirSync(other)
  execFileSync("truncate", ["-s", "12G", image])
  execFileSync("mkfs.ext4", ["-q", "-m", "0", "-F", image])
  const device = execFileSync("losetup", ["-f", "--show", image], {encoding: "utf8"}).trim()
  const volumes = [work, other], results = volumes.map(root => join(root, "writer-done"))
  try {
    execFileSync("mount", [device, other])
    for (const root of volumes) writeFileSync(join(root, "disk-wait.py"), `import os, time
with open("writer-output", "wb") as out:
    for _ in range(64):
        out.write(b"x" * (16 << 20)); out.flush(); os.fsync(out.fileno()); time.sleep(0.125)
    time.sleep(2)
os.unlink("writer-output")
open("writer-done", "w").write("DONE")
`)
    const submit = n => ask(`Run exactly cd "${volumes[n]}" && exec python3 disk-wait.py once. Do not change it, retry it or start it elsewhere if it waits.`, agents[n])
    const waiting = n => ctx.latestRows().find(row => row.owner?.id === agents[n].id && (row.state?.command_line ?? row.command)?.includes("disk-wait.py") && row.state?.kind === "held" && row.state.reason === "disk")
    await Promise.all(agents.map((_, n) => submit(n)))
    await until(() => results.every(path => existsSync(path)), "both warmup writers to finish", 180000)
    // Wait for the real provider turns and process watches to observe exit.
    await until(() => ctx.latestRows().every(row => !(row.state?.command_line ?? row.command)?.includes("disk-wait.py")), "warmup writer rows to leave", 30000)
    for (const [n, root] of volumes.entries()) {
      rmSync(results[n])
      const line = execFileSync("df", ["-B1", "--output=avail", root], {encoding: "utf8"}).trim().split("\n")[1]
      execFileSync("fallocate", ["-l", String(Number(line.trim()) - 10 * GIB + 32 * 1024 ** 2), join(root, "held-space")])
    }
    await submit(0)
    await until(() => waiting(0), "the first writer held for disk", 120000)
    await submit(1)
    await until(() => waiting(1), "the second writer held for disk", 120000)
    report.disk_waits_before_recovery = [waiting(0), waiting(1)]
    note("both-volumes-held", {pids: report.disk_waits_before_recovery.map(row => row.pid)})
    rmSync(join(other, "held-space"))
    const resumed = await until(() => existsSync(results[1]), "the later writer to finish on its recovered volume", 45000).then(() => true, () => false)
    check("recovering only the later writer's volume lets it finish", resumed)
    check("the first writer stays held on its full volume", !!waiting(0))
    rmSync(join(work, "held-space"))
    await until(() => results.every(path => existsSync(path)), "both writers to finish after both volumes recover", 45000)
    check("both writers finish after their own volumes recover", true)
    assert(resumed, "the recovered later writer remained blocked")
  } finally {
    // Free only this run's filler files before the harness stops its owners.
    for (const root of volumes) rmSync(join(root, "held-space"), {force: true})
    try { execFileSync("umount", [other]) } catch { execFileSync("umount", ["-l", other]) }
    execFileSync("losetup", ["-d", device])
    rmSync(image)
  }
}
