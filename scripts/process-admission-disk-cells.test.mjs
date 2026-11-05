import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

for (const valid of [false, true]) {
  test(`a completed temp scenario with ${valid ? "passing" : "failing"} checks exits ${valid ? "zero" : "nonzero"}`, () => {
    const dir = mkdtempSync(join(tmpdir(), "disk-cell-checks-"))
    try {
      const result = spawnSync(process.execPath, ["--input-type=module", "-e", `
        import { runDiskCells } from ${JSON.stringify(new URL("./drills/process-admission-disk-cells.mjs", import.meta.url).href)}
        import { mkdirSync, writeFileSync } from "node:fs"
        import { join } from "node:path"
        const dir = ${JSON.stringify(dir)}
        const folder = join(dir, "chariox/t/room/001")
        mkdirSync(folder, { recursive: true, mode: 0o700 })
        const actual = ${valid} ? folder : "/wrong-temp-root"
        writeFileSync(join(dir, "answer-temp-b1.txt"), [
          "TMPDIR=" + actual + " TMP=" + actual + " TEMP=" + actual,
          "mktemp=" + actual + "/file", "python=" + actual + "/directory",
          "INSTR=Use TMPDIR; temporary files are cleared when idle", "DONE",
        ].join("\\n"))
        const report = {}
        try {
          await runDiskCells({
            scenario: "temp", agents: [{ id: "agent-001" }],
            ask: async () => {}, note: (name, detail) => console.log(JSON.stringify({ name, ...detail })),
            until: async predicate => assertPredicate(predicate),
            work: dir, dir, sessionId: "room", report,
            flags: new Map(), providerOf: () => "test-provider",
          })
        } finally {
          console.log(JSON.stringify({ report }))
        }
        function assertPredicate(predicate) {
          if (!predicate()) throw new Error("fixture answer missing")
        }
      `], { encoding: "utf8", timeout: 10000 })
      assert.ifError(result.error)
      const rows = result.stdout.trim().split("\n").map(line => JSON.parse(line))
      const done = rows.find(row => row.name === "scenario-done")
      assert(done, "the scenario completed and recorded its checks")
      const report = rows.find(row => row.report)?.report
      assert(report.checks.length > 0)
      if (valid) {
        assert.equal(result.status, 0, result.stderr)
        assert(report.checks.every(check => check.ok))
        assert.deepEqual(done.failed, [])
      } else {
        assert(report.checks.some(check => !check.ok), "the scenario recorded a deliberate failure")
        assert.notEqual(result.status, 0, "the executable reported success despite failed checks")
        assert.match(result.stderr, /TMPDIR is its own folder/)
      }
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
}
