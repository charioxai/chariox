#!/usr/bin/env node
// never-seen: scan the places a secret must never reach, with a needle this tool never prints.
//   node never-seen.mjs --roots roots.json (--needle-stdin | --needle-file F) [--ocr] [--live-control-file F --live-control-targets T1,T2] --out scan.json
// Exit 0 CLEAN, 1 LEAK, 2 INVALID. Output holds counts, hashes, and file+offset only.
import { readFile, writeFile } from "node:fs/promises"
import { syntheticControl } from "./control.mjs"
import { NeedleRefused, needleHash, needleSet } from "./needles.mjs"
import { ocrAvailable, scanRoots } from "./scan.mjs"

function parseArgs(argv) {
  const flags = new Map()
  for (let i = 0; i < argv.length; i++) { const key = argv[i]; if (!key.startsWith("--")) throw new Error(`unexpected argument ${key}`); const next = argv[i + 1]; if (next === undefined || next.startsWith("--")) flags.set(key, true); else { flags.set(key, next); i++ } }
  return flags
}

async function readStdin() { const chunks = []; for await (const c of process.stdin) chunks.push(c); return Buffer.concat(chunks).toString("utf8").replace(/\r?\n$/, "") }

const countBy = (roots, key = "target") => { const out = {}; for (const r of roots) out[r[key]] = (out[r[key]] ?? 0) + r.hits.length; return out }

/** Pure verdict, exported for the unit test. */
export function verdictOf({ roots, syntheticOk, liveControl, ocr }) {
  const reasons = []
  if (!syntheticOk) reasons.push("synthetic control did not find the planted needle in every file type")
  for (const r of roots) if (r.files === 0) reasons.push(`root ${r.target} (${r.path}) has 0 files`)
  if (liveControl) for (const [target, n] of Object.entries(liveControl.hits)) if (n === 0) reasons.push(`live control not found in ${target}`)
  if (ocr.requested && !ocr.available) reasons.push("OCR requested but tesseract is not available")
  if (reasons.length) return { verdict: "INVALID", reasons }
  const leaks = roots.reduce((n, r) => n + r.hits.length, 0)
  return { verdict: leaks ? "LEAK" : "CLEAN", reasons: [] }
}

async function main() {
  const flags = parseArgs(process.argv.slice(2))
  const roots = JSON.parse(await readFile(flags.get("--roots"), "utf8"))
  let value
  if (flags.get("--needle-stdin")) value = await readStdin()
  else if (flags.get("--needle-file")) value = (await readFile(flags.get("--needle-file"), "utf8")).replace(/\r?\n$/, "")
  else throw new Error("give --needle-stdin or --needle-file")
  let needles
  try { needles = needleSet(value) } catch (error) { if (error instanceof NeedleRefused) { process.stderr.write(`refused: ${error.message}\n`); process.exit(2) } throw error }
  const wantOcr = flags.get("--ocr") === true
  const synthetic = await syntheticControl({ ocr: wantOcr })
  const real = await scanRoots(roots, needles, { ocr: wantOcr })
  let liveControl = null
  if (flags.get("--live-control-file")) {
    const control = (await readFile(flags.get("--live-control-file"), "utf8")).trim()
    const targets = String(flags.get("--live-control-targets") ?? "").split(",").filter(Boolean)
    const scanned = await scanRoots(roots.filter(r => targets.includes(r.target)), needleSet(control), { ocr: wantOcr })
    liveControl = { hits: Object.fromEntries(targets.map(t => [t, scanned.roots.filter(r => r.target === t).reduce((n, r) => n + r.hits.length, 0)])) }
  }
  const syntheticOk = Object.values(synthetic.types).every(Boolean) && synthetic.ocr !== false
  const { verdict, reasons } = verdictOf({ roots: real.roots, syntheticOk, liveControl, ocr: { requested: wantOcr, available: wantOcr ? await ocrAvailable() : false } })
  const scan = {
    run: flags.get("--run") ?? null, build_pin: flags.get("--build-pin") ?? null, needle_sha8: needleHash(value),
    roots: real.roots.map(r => ({ target: r.target, kernel: r.kernel, path: r.path, files: r.files, bytes: r.bytes })),
    control: { synthetic: synthetic.types, live: liveControl?.hits ?? null },
    real: countBy(real.roots),
    matches: real.roots.flatMap(r => r.hits.map(h => ({ target: r.target, path: h.path, offset: h.offset, form: h.form, via: h.via ?? "bytes" }))),
    ocr: { requested: wantOcr, images: real.roots.reduce((n, r) => n + r.images, 0), ocr_ok: real.roots.reduce((n, r) => n + r.ocr_ok, 0), not_scanned: real.roots.reduce((n, r) => n + r.not_scanned, 0) },
    verdict, reasons,
  }
  await writeFile(flags.get("--out") ?? "scan.json", JSON.stringify(scan, null, 2) + "\n", { mode: 0o600 })
  process.stdout.write(`${verdict} needle#${scan.needle_sha8} real=${JSON.stringify(scan.real)}\n`)
  process.exit(verdict === "CLEAN" ? 0 : verdict === "LEAK" ? 1 : 2)
}

if (import.meta.url === `file://${process.argv[1]}`) main().catch(error => { process.stderr.write(`never-seen: ${error.message}\n`); process.exit(2) })
