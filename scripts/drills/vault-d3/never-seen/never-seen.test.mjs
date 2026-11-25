import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, mkdir, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { syntheticControl } from "./control.mjs"
import { NeedleRefused, needleSet } from "./needles.mjs"
import { verdictOf } from "./never-seen.mjs"
import { scanFile, scanRoots } from "./scan.mjs"

const here = dirname(fileURLToPath(import.meta.url))
const secret = "S3cr3t-Token_" + "abcdef0123456789"

test("refuses empty, short and one-character needles", () => {
  for (const bad of ["", "short", "a".repeat(40)]) assert.throws(() => needleSet(bad), NeedleRefused)
})

test("needle set has raw, base64, base64url, hex, url, json and the first and last 12", () => {
  const names = needleSet(secret + '"/+').map(n => n.name)
  for (const name of ["raw", "base64", "base64url", "hex", "url", "json", "first12", "last12"]) assert.ok(names.includes(name), name)
})

test("synthetic control finds the planted needle in every file type", async () => {
  const { types } = await syntheticControl({ ocr: false })
  for (const [type, ok] of Object.entries(types)) assert.ok(ok, type)
})

test("a planted leak is found, with offset and form only, across the read-chunk boundary", async () => {
  const dir = await mkdtemp(join(tmpdir(), "ns-test-"))
  try {
    const file = join(dir, "leak.log")
    await writeFile(file, Buffer.concat([Buffer.alloc(4 * 1024 * 1024 - 5, 0x20), Buffer.from(secret)]))
    const found = await scanFile(file, needleSet(secret))
    assert.equal(found.hits.filter(h => h.form === "raw").length, 1)
    assert.equal(found.hits.find(h => h.form === "raw").offset, 4 * 1024 * 1024 - 5)
    for (const hit of found.hits) assert.deepEqual(Object.keys(hit).sort(), ["form", "offset"])
  } finally { await rm(dir, { recursive: true, force: true }) }
})

test("encoded copies are found; symlinks and vault/config names are skipped", async () => {
  const dir = await mkdtemp(join(tmpdir(), "ns-test-"))
  try {
    await writeFile(join(dir, "b64.json"), JSON.stringify({ x: Buffer.from(secret).toString("base64") }))
    await writeFile(join(dir, "vault.db"), secret)
    await writeFile(join(dir, "auth.json"), secret)
    await mkdir(join(dir, "real")); await writeFile(join(dir, "real", "a.txt"), secret)
    await symlink(join(dir, "real"), join(dir, "link"))
    const { roots } = await scanRoots([{ target: "T3", kind: "dir", path: dir }], needleSet(secret))
    const names = roots[0].hits.map(h => h.path.replace(dir + "/", ""))
    assert.ok(names.includes("b64.json"))
    assert.ok(names.includes("real/a.txt"))
    assert.ok(!names.some(n => n.startsWith("link") || n === "vault.db" || n === "auth.json"), names.join())
  } finally { await rm(dir, { recursive: true, force: true }) }
})

test("verdicts: clean, leak, and invalid when a control or a root is empty", () => {
  const ok = { roots: [{ target: "T1", path: "/x", files: 3, hits: [] }], syntheticOk: true, liveControl: { hits: { T1: 2 } }, ocr: { requested: false, available: false } }
  assert.equal(verdictOf(ok).verdict, "CLEAN")
  assert.equal(verdictOf({ ...ok, roots: [{ ...ok.roots[0], hits: [{}] }] }).verdict, "LEAK")
  assert.equal(verdictOf({ ...ok, syntheticOk: false }).verdict, "INVALID")
  assert.equal(verdictOf({ ...ok, liveControl: { hits: { T1: 0 } } }).verdict, "INVALID")
  assert.equal(verdictOf({ ...ok, roots: [{ ...ok.roots[0], files: 0 }] }).verdict, "INVALID")
  assert.equal(verdictOf({ ...ok, ocr: { requested: true, available: false } }).verdict, "INVALID")
})

test("CLI: leak exits 1, clean exits 0, short needle exits 2, and no output holds the value", async () => {
  const dir = await mkdtemp(join(tmpdir(), "ns-test-"))
  try {
    await mkdir(join(dir, "logs")); await writeFile(join(dir, "logs", "a.log"), "nothing here\n")
    const rootsFile = join(dir, "roots.json")
    await writeFile(rootsFile, JSON.stringify([{ target: "T3", kind: "dir", path: join(dir, "logs") }]))
    const run = (input, out) => spawnSync("node", [join(here, "never-seen.mjs"), "--roots", rootsFile, "--needle-stdin", "--out", join(dir, out)], { input, encoding: "utf8" })
    const clean = run(secret, "clean.json")
    assert.equal(clean.status, 0, clean.stdout + clean.stderr)
    await writeFile(join(dir, "logs", "leak.log"), `oops ${secret}\n`)
    const leak = run(secret, "leak.json")
    assert.equal(leak.status, 1)
    assert.equal(run("short", "short.json").status, 2)
    for (const text of [clean.stdout, clean.stderr, leak.stdout, leak.stderr, await (await import("node:fs/promises")).readFile(join(dir, "leak.json"), "utf8")]) assert.ok(!text.includes(secret))
  } finally { await rm(dir, { recursive: true, force: true }) }
})

test("process args and environment are scanned (planted control in a child's environ and argv)", async () => {
  const { spawn } = await import("node:child_process")
  const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 30000)", `marker=${secret}`], { env: { ...process.env, PLANTED_CONTROL: secret }, stdio: "ignore" })
  try {
    await new Promise(resolve => child.once("spawn", resolve)); await new Promise(resolve => setTimeout(resolve, 300))
    const { roots } = await scanRoots([{ target: "T9", kind: "proc" }], needleSet(secret))
    const mine = roots[0].hits.filter(h => h.path.includes(`pid ${child.pid}`))
    assert.ok(mine.some(h => h.path.endsWith("cmdline")) && mine.some(h => h.path.endsWith("environ")))
  } finally { child.kill() }
})

test("OCR control: a value drawn in a screenshot and inside a provider jsonl image block is found", async t => {
  const { ocrAvailable } = await import("./scan.mjs")
  if (!(await ocrAvailable())) return t.skip("tesseract not installed: images would be 'not scanned' and the cell BLOCKED")
  const { types, ocr } = await syntheticControl({ ocr: true })
  assert.equal(ocr, true, JSON.stringify(types))
})

test("surface matrix: each surface finds its own live decoy, a real secret leaking into one surface is named, a blind surface is BLOCKED", async () => {
  const { surfaceMatrix, surfaceVerdicts } = await import("./surface-scan.mjs")
  const dir = await mkdtemp(join(tmpdir(), "ns-surf-"))
  try {
    const decoyA = "DECOY_logs_0123456789abcdef", decoyB = "DECOY_work_fedcba9876543210", decoyC = "DECOY_blind_00112233445566", real = "REAL_secret_value_1234567890"
    for (const d of ["logs", "work", "blind"]) await mkdir(join(dir, d))
    await writeFile(join(dir, "logs", "a.ndjson"), JSON.stringify({ m: decoyA }) + "\n")
    await writeFile(join(dir, "work", "note.txt"), `${decoyB}\nleak: ${real}\n`)
    await writeFile(join(dir, "blind", "x.log"), "nothing planted here\n")
    const surfaces = ["logs", "work", "blind"], groups = [{ label: "dA", value: decoyA, role: "decoy" }, { label: "dB", value: decoyB, role: "decoy" }, { label: "dC", value: decoyC, role: "decoy" }, { label: "real", value: real, role: "real" }]
    const { matrix } = await surfaceMatrix({ surfaces: surfaces.map(label => ({ label, path: join(dir, label) })), groups })
    const v = Object.fromEntries(surfaceVerdicts({ surfaces, matrix, groups, decoyOf: { logs: "dA", work: "dB", blind: "dC" } }).map(x => [x.surface, x]))
    assert.equal(v.logs.verdict, "PASS")
    assert.equal(v.work.verdict, "FAIL"); assert.equal(v.work.leaks[0].group, "real")
    assert.equal(v.blind.verdict, "BLOCKED")
    assert.ok(!JSON.stringify(matrix).includes(real))
  } finally { await rm(dir, { recursive: true, force: true }) }
})

// Review findings on #1743 (scanner evidence must be complete, positive hits must never be hidden).
test("many decoy matches in one file do not hide a real secret in the same file", async () => {
  const { surfaceMatrix, surfaceVerdicts } = await import("./surface-scan.mjs")
  const dir = await mkdtemp(join(tmpdir(), "ns-cap-"))
  try {
    const decoy = "DECOY_chat_0123456789abcdef", real = "REAL_secret_value_1234567890"
    await mkdir(join(dir, "logs"))
    await writeFile(join(dir, "logs", "a.log"), (decoy + "\n").repeat(40) + `${real}\n`)
    const groups = [{ label: "d", value: decoy, role: "decoy" }, { label: "real", value: real, role: "real" }]
    const { matrix } = await surfaceMatrix({ surfaces: [{ label: "logs", path: join(dir, "logs") }], groups })
    assert.ok(matrix.real?.logs, "the real value is found after more than 20 decoy hits")
    assert.equal(surfaceVerdicts({ surfaces: ["logs"], matrix, groups, decoyOf: { logs: "d" } })[0].verdict, "FAIL")
  } finally { await rm(dir, { recursive: true, force: true }) }
})

test("a file the scanner cannot read makes the root incomplete and the verdict INVALID / BLOCKED", async () => {
  const { surfaceMatrix, surfaceVerdicts } = await import("./surface-scan.mjs")
  const dir = await mkdtemp(join(tmpdir(), "ns-unread-"))
  try {
    await mkdir(join(dir, "logs")); await writeFile(join(dir, "logs", "ok.log"), "DECOY_x_0123456789abcdef\n"); await writeFile(join(dir, "logs", "locked.log"), "x")
    const { scanFile } = await import("./scan.mjs")
    const reader = (path, needles) => path.endsWith("locked.log") ? Promise.reject(new Error("EACCES")) : scanFile(path, needles)
    const groups = [{ label: "d", value: "DECOY_x_0123456789abcdef", role: "decoy" }, { label: "real", value: "REAL_secret_value_1234567890", role: "real" }]
    const { matrix, roots } = await surfaceMatrix({ surfaces: [{ label: "logs", path: join(dir, "logs") }], groups, reader })
    assert.equal(roots[0].unreadable, 1)
    const v = surfaceVerdicts({ surfaces: ["logs"], matrix, groups, decoyOf: { logs: "d" }, coverage: { logs: roots[0] } })[0]
    assert.equal(v.verdict, "BLOCKED"); assert.match(v.reason, /unreadable/)
    assert.equal(verdictOf({ roots: [{ target: "T3", path: "/x", files: 1, hits: [], unreadable: 1 }], syntheticOk: true, liveControl: null, ocr: { requested: false, available: false } }).verdict, "INVALID")
  } finally { await rm(dir, { recursive: true, force: true }) }
})

test("an image OCR cannot decode is not scanned: the OCR call rejects and the verdicts are not clean", async t => {
  const { ocrAvailable, ocrBuffer } = await import("./scan.mjs")
  if (!(await ocrAvailable())) return t.skip("tesseract not installed")
  const corrupt = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.from("not a real png body")])
  await assert.rejects(ocrBuffer(corrupt))
  const dir = await mkdtemp(join(tmpdir(), "ns-ocr-"))
  try {
    await writeFile(join(dir, "shot.png"), corrupt)
    const { roots } = await scanRoots([{ target: "T6", kind: "dir", path: dir }], needleSet(secret), { ocr: true })
    assert.equal(roots[0].images, 1); assert.equal(roots[0].not_scanned, 1); assert.equal(roots[0].ocr_ok, 0)
    assert.equal(verdictOf({ roots, syntheticOk: true, liveControl: null, ocr: { requested: true, available: true } }).verdict, "INVALID")
  } finally { await rm(dir, { recursive: true, force: true }) }
})

test("a real secret hit is a FAIL even on a surface without a decoy; a missing expected needle blocks every PASS", async () => {
  const { surfaceMatrix, surfaceVerdicts } = await import("./surface-scan.mjs")
  const dir = await mkdtemp(join(tmpdir(), "ns-fail-"))
  try {
    const real = "REAL_secret_value_1234567890", decoy = "DECOY_ok_0123456789abcdef"
    for (const d of ["blind", "ok"]) await mkdir(join(dir, d))
    await writeFile(join(dir, "blind", "x.log"), `leak ${real}\n`); await writeFile(join(dir, "ok", "x.log"), `${decoy}\n`)
    const groups = [{ label: "d", value: decoy, role: "decoy" }, { label: "real", value: real, role: "real" }]
    const { matrix } = await surfaceMatrix({ surfaces: ["blind", "ok"].map(label => ({ label, path: join(dir, label) })), groups })
    const v = Object.fromEntries(surfaceVerdicts({ surfaces: ["blind", "ok"], matrix, groups, decoyOf: { ok: "d" } }).map(x => [x.surface, x]))
    assert.equal(v.blind.verdict, "FAIL"); assert.equal(v.blind.leaks[0].group, "real"); assert.equal(v.ok.verdict, "PASS")
    const v2 = Object.fromEntries(surfaceVerdicts({ surfaces: ["ok"], matrix, groups, decoyOf: { ok: "d" }, expectedReal: ["real", "real-capture-token"] }).map(x => [x.surface, x]))
    assert.equal(v2.ok.verdict, "BLOCKED"); assert.match(v2.ok.reason, /real-capture-token/)
  } finally { await rm(dir, { recursive: true, force: true }) }
})
