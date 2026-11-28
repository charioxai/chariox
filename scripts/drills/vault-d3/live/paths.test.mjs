import assert from "node:assert/strict"
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import test from "node:test"
import { kernelBrowserScript, kernelStateSurfaces } from "./paths.mjs"
import { surfaceMatrix, surfaceVerdicts } from "../never-seen/surface-scan.mjs"

test("durable database and WAL are scanned while Vault stays excluded", async () => {
  const dir = mkdtempSync(join(tmpdir(), "d3-paths-"))
  try {
    const home = join(dir, "chariox"), state = join(home, "kernels", "test-kernel")
    mkdirSync(state, { recursive: true })
    mkdirSync(join(home, "state"))
    const db = join(state, "state.db")
    const planted = "D3SyntheticControl_0123456789"
    const created = spawnSync("python3", ["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('create table control(value text)'); c.execute('insert into control values (?)',(sys.argv[2],)); c.commit(); c.close()", db, planted], { encoding: "utf8" })
    assert.equal(created.status, 0, "synthetic database creation succeeds")
    writeFileSync(db + "-wal", planted)
    writeFileSync(join(state, "vault.db"), planted)
    const groups = [{ label: "control", role: "real", value: planted }]
    const { matrix, hitFiles } = await surfaceMatrix({ surfaces: kernelStateSurfaces(home), groups })
    const files = Object.values(hitFiles.control ?? {}).flat()
    assert.ok(files.includes(db), "durable database must be scanned")
    assert.ok(files.includes(db + "-wal"), "durable WAL must be scanned")
    assert.ok(!files.includes(join(state, "vault.db")), "Vault remains excluded")
    const verdicts = surfaceVerdicts({ surfaces: Object.keys(matrix.control), matrix, groups, decoyOf: {} })
    assert.ok(verdicts.every(v => v.verdict === "FAIL"), "real hits fail without waiting for a decoy")
    assert.ok(!JSON.stringify({ matrix, hitFiles, verdicts }).includes(planted), "only counts and positions leave the scanner")
  } finally { rmSync(dir, { recursive: true, force: true }) }
})

test("browser override resolves a directory and validates entry scripts before launch", () => {
  const dir = mkdtempSync(join(tmpdir(), "d3-browser-path-"))
  try {
    assert.equal(kernelBrowserScript(undefined), undefined)
    assert.throws(() => kernelBrowserScript(dir), /kernel-browser-host/)
    const entry = join(dir, "kernel-browser-host.mjs")
    writeFileSync(entry, "// synthetic controller entry")
    assert.equal(kernelBrowserScript(dir), entry)
    assert.equal(kernelBrowserScript(entry), entry)
    assert.throws(() => kernelBrowserScript(join(dir, "missing")), /D3_KERNEL_SCRIPT/)
  } finally { rmSync(dir, { recursive: true, force: true }) }
})
