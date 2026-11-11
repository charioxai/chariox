// The file-bytes bar (MP-08..MP-11): the verified file bytes of a transfer over the bytes the relay put on the shaped
// link for it must be at least 90 %. Runs run.mjs for `files get` and `files put` --repeat times each, one at a time,
// and prints the share of every run and the median per direction. Load-sensitive in time, not in bytes: the share is
// a ratio of byte counts, so a loaded host changes the elapsed time but not the result.
//   node file-share.mjs --kernel-binary K --relay-binary R --evidence-dir DIR --label base [--repeat 3] [--size-mb 30]
import { spawnSync } from "node:child_process"
import { readFileSync, writeFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const here = dirname(fileURLToPath(import.meta.url))
const flags = new Map()
for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return v }
const evidence = resolve(need("--evidence-dir")), label = need("--label")
const repeat = Number(flags.get("--repeat") ?? 3), sizeMb = flags.get("--size-mb") ?? "30"
const runs = { get: "8m", put: "8m-sym" }
const median = values => { const sorted = [...values].sort((a, b) => a - b); return sorted[Math.floor(sorted.length / 2)] }
const summary = {}
for (const [direction, profile] of Object.entries(runs)) {
  summary[direction] = []
  for (let i = 1; i <= repeat; i += 1) {
    const runLabel = `${label}-${direction}-run${i}`
    const run = spawnSync("node", [join(here, "run.mjs"), "--kernel-binary", need("--kernel-binary"), "--relay-binary", need("--relay-binary"), "--evidence-dir", evidence,
      "--label", runLabel, "--profile", profile, "--direction", direction, "--variant", "single", "--size-mb", sizeMb, "--state-root", flags.get("--state-root") ?? "/root/.chariox/dev/relay8/runs"], { stdio: "inherit" })
    const result = JSON.parse(readFileSync(join(evidence, `${runLabel}-${profile}-single`, "result.json"), "utf8"))
    summary[direction].push({ run: i, exit: run.status, fileShare: result.fileShare, fileBytes: result.fileBytes, wireBytes: result.wire?.downBytes, elapsedMs: result.transfers?.[0]?.elapsedMs, ok: result.transfers?.every(t => t.code === 0 && t.hashOk) === true })
  }
}
const lines = [`# File bytes over wire bytes, ${label} (${sizeMb} MiB, ${repeat} runs each, RTT 60 ms)`, "", "| direction | run | file bytes | wire bytes | share | elapsed s | transfer ok |", "|---|---|---|---|---|---|---|"]
for (const [direction, rows] of Object.entries(summary)) for (const r of rows) lines.push(`| ${direction} | ${r.run} | ${r.fileBytes} | ${r.wireBytes} | ${(r.fileShare * 100).toFixed(1)}% | ${(r.elapsedMs / 1000).toFixed(1)} | ${r.ok} |`)
lines.push("", "| direction | median share | bar 90% |", "|---|---|---|")
for (const [direction, rows] of Object.entries(summary)) { const m = median(rows.filter(r => r.ok).map(r => r.fileShare)); lines.push(`| ${direction} | ${(m * 100).toFixed(1)}% | ${m >= 0.9 ? "PASS" : "FAIL"} |`) }
writeFileSync(join(evidence, `${label}-file-share.md`), lines.join("\n") + "\n")
writeFileSync(join(evidence, `${label}-file-share.json`), JSON.stringify(summary, null, 2) + "\n")
console.log(lines.join("\n"))
