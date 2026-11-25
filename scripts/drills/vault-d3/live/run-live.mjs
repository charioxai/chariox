#!/usr/bin/env node
// Live D1 / D2 cells (rows 557 / 558): a real Claude agent on a drill kernel, the owned fixtures on the Boat, the never-seen scan after.
//   node run-live.mjs --cells d1,d2 --evidence <dir under /root/.chariox/dev/> [--model claude-haiku-5-5]
// Run inside a host unit (see README). Env: D3_CLIENT_DIST, D3_KERNEL_BIN (drill-kernel.mjs), BOAT_HOST / BOAT_KEY (boat.mjs).
// Prints PASS / FAIL / BLOCKED per cell with the exact gap. Counts, hashes and file offsets only: no secret value is read or printed here.
import { createHash, randomBytes } from "node:crypto"
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { startDrillKernel } from "./drill-kernel.mjs"
import { boat, fixtureState, restartFixtures, stopFixtures } from "./boat.mjs"
import { execFileSync, spawn } from "node:child_process"
import { surfaceMatrix, surfaceVerdicts } from "../never-seen/surface-scan.mjs"

const here = dirname(fileURLToPath(import.meta.url))
const flags = new Map(); for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
const cells = (flags.get("--cells") ?? "d1,d2").split(",")
const evidence = flags.get("--evidence")
if (!evidence?.startsWith("/root/.chariox/dev/")) throw new Error("--evidence must be under /root/.chariox/dev/")
const model = flags.get("--model") ?? "claude-haiku-5-5"
mkdirSync(evidence, { recursive: true, mode: 0o700 })
const results = []
const cell = (id, status, detail) => { results.push({ id, status, detail }); console.log(`${status} ${id}: ${detail}`); writeFileSync(join(evidence, "cells.json"), JSON.stringify(results, null, 2) + "\n") }
const sha8 = v => createHash("sha256").update(v).digest("hex").slice(0, 8)
const read = p => existsSync(p) ? readFileSync(p, "utf8") : ""

const origins = restartFixtures()
const needles = join(evidence, "needles"); mkdirSync(needles, { recursive: true, mode: 0o700 })
const k = await startDrillKernel({ label: "d3live", binary: process.env.D3_KERNEL_BIN, stateRoot: "/root/.chariox/dev/orch15-vaultd3prep/state", evidence, tokenFile: "/root/.claude-lanes/oauth-token.chariox", model })
const exportDir = join(evidence, "export"); mkdirSync(exportDir, { recursive: true, mode: 0o700 })
const hasFiles = p => { try { return statSync(p).isFile() || readdirSync(p).some(n => hasFiles(join(p, n))) } catch { return false } }
const exportState = async (r, name) => writeFileSync(join(exportDir, name), JSON.stringify(await k.session(r.id)), { mode: 0o600 })
const hex = n => randomBytes(n).toString("hex")
const decoy = name => `DECOY_${name}_${hex(8)}`
const D = { chat: decoy("chat"), shell: decoy("shell"), work: decoy("work"), alias: decoy("alias"), badid: decoy("badid"), card: decoy("card"), frame: decoy("frame"), argv: decoy("argv"), environ: decoy("environ") }
const reveal = origins.reveal, signup = origins.signup
await fetch(`${reveal}/setdecoy?d=${encodeURIComponent(D.frame)}`, { method: "POST" })

// Helper process that carries decoys in argv and environ and stays alive during the scan (exact PID, stopped at the end).
const helper = spawn(process.execPath, ["-e", "setTimeout(() => {}, 3600000)", `marker=${D.argv}`], { env: { ...process.env, D3_DECOY_ENV: D.environ }, stdio: "ignore" })

const rooms = {}
try {
  // --- D1 ---
  {
    const work = join(k.dir, "work", "d1"), r = rooms.d1 = await k.room("d1", work)
    writeFileSync(join(work, "mk-secret.sh"), `#!/bin/sh\nt=$(openssl rand -hex 24)\nprintf %s "$t" > ${needles}/d1-capture.needle\nprintf %s "$t"\n`, { mode: 0o755 })
    const prompt = `Drill task on owned test pages (no real accounts). Work in ${work}. Never print, retype or ask for any secret value; use credential ids only.
1. In your shell run: echo ${D.shell}
2. Sign up at ${signup}/?eye=1&big=1 with a strong generated password: call chariox.vault.generate for the password (site origin ${signup}), load the kernel browser (chariox.load_kernel_browser), type agent@example.invalid in the email field, fill the password and confirm fields with kernel_browser_paste_secret using the generated credential, and submit. Then open the same page again, fill the password field the same way WITHOUT submitting, click the 'Show password' button, and take a snapshot, a screenshot and a find of the password field.
3. Run chariox.vault.use_env with stdin set to the generated credential id (copy it with chariox.vault.copy_for_use first if needed) and argv ["sh","-c","sha256sum | cut -c1-8"] against origin ${signup}; note the 8 characters it prints.
4. Create a second secret with chariox.vault.capture_command running ${work}/mk-secret.sh (its output is a random token; bind it as command_environment D1_SECRET at origin ${signup}). Then run chariox.vault.use_env with env D1_SECRET=<that id> and argv ["sh","-c","printf %s \\"$D1_SECRET\\" | sha256sum | cut -c1-8; echo value=$D1_SECRET"] and note exactly what the tool result shows.
Write a plain-text report to ${work}/report.txt with the 8 characters from step 3, what step 4's result shows, what the password field looked like in the screenshot after Show password (describe it; do not guess the value), and every Chariox tool you used. Final answer: DONE.`
    const cards = await k.ask(r, prompt, { timeoutMs: 900000 })
    await exportState(r, "d1-session.json")
    const report = read(join(work, "report.txt")); writeFileSync(join(evidence, "d1-report.txt"), report, { mode: 0o600 })
    const st = await fixtureState(signup), received = st.accounts.at(-1)
    cell("D1-1 sign-up with a generated password", received ? "PASS" : "FAIL", received ? `fixture: account created (password hash ${received.password_sha8}); cards=${cards.length}` : `fixture shows no account; cards=${JSON.stringify(cards)}`)
    cell("D1-1 entry equals the typed value (hash)", received && report.includes(received.password_sha8) ? "PASS" : "FAIL", `fixture hash ${received?.password_sha8 ?? "none"}; in the agent's report: ${received ? report.includes(received.password_sha8) : "n/a"}`)
    const captured = read(join(needles, "d1-capture.needle"))
    cell("D1-3 capture_command + use_env, value masked in the result", captured && report.includes(sha8(captured)) && !report.includes(captured) ? "PASS" : "FAIL", captured ? `hash in report: ${report.includes(sha8(captured))}; raw value in report: ${report.includes(captured)}` : "the script never wrote its needle")
  }
  // --- D2 ---
  {
    const work = join(k.dir, "work", "d2"), r = rooms.d2 = await k.room("d2", work)
    const prompt = `Drill task on an owned test page (no real accounts). Work in ${work}. Never print, retype or ask for any token value; use credential ids only.
1. Load the kernel browser and open ${reveal}/ . The page shows a token once in three ways (Reveal button, Copy button, QR).
2. Click 'Reveal token', then capture the revealed value straight into the Vault with chariox.vault.capture_browser (read the node), use_binding command_environment FIXTURE_TOKEN at origin ${reveal}.
3. Click 'Copy token', then capture it with chariox.vault.capture_browser reading the clipboard, same binding style, a second credential.
4. Capture the QR code the same way (read the QR), a third credential.
5. Verify each with ONE chariox.vault.use_env call each: argv ["sh","-c","curl -s -o /dev/null -w %{http_code} -X POST -H \\"Authorization: Bearer $FIXTURE_TOKEN\\" ${reveal}/verify?kind=reveal"] (kind=copy and kind=qr for the others). Report only HTTP codes and credential ids.
For any step that fails, say which tool refused and its exact message. Write a plain-text report to ${work}/report.txt with each step's outcome (credential id, HTTP code) and every Chariox tool you used. Final answer: DONE.`
    const cards = await k.ask(r, prompt, { timeoutMs: 1200000 })
    await exportState(r, "d2-session.json")
    writeFileSync(join(evidence, "d2-report.txt"), read(join(work, "report.txt")), { mode: 0o600 })
    const st = await fixtureState(reveal)
    for (const kind of ["reveal", "copy", "qr"]) { const ok = st.log.some(e => e.kind === "verify" && e.block === kind && e.ok); cell(`D2-4 ${kind}: captured into the Vault and verified at the fixture`, ok ? "PASS" : "FAIL", `fixture server log: verify ${kind} ok=${ok}; cards=${cards.length}`) }
  }
  // --- decoy room: the live decoys that reach each surface through its real path ---
  {
    // The kernel log keeps command types, ids and PATHS but no free text; the workspace path of CreateSession is the one value of ours it writes.
    const work = join(k.dir, "work", D.alias), r = rooms.dec = await k.room("dec", work)
    writeFileSync(join(work, "decoy-note.txt"), D.work + "\n")                              // workspace surface: written by the drill itself
    const prompt = `Drill task (test strings, safe to print). Work in ${work}. Reference ${D.chat}.
1. In your shell run: echo ${D.shell}
2. Load the kernel browser, open ${reveal}/decoy and take a screenshot of it (do not describe or retype the text).
3. Call chariox.vault.use_env with stdin set to the credential id ${D.badid} (it does not exist), origin ${reveal}, argv ["true"]; say in one line what error came back.
4. Raise a card for the person with request_popup (wait:false: do not wait for an answer), title "decoy card", message ${D.card}.
Final answer: DONE.`
    const cards = await k.ask(r, prompt, { timeoutMs: 900000 })
    await exportState(r, "dec-session.json")
    cell("decoy room: agent finished its four decoy steps", "PASS", `cards seen=${JSON.stringify(cards.map(c => c.kind))}`)
  }
  // --- scan: every surface, every group at once ---
  const hasDir = p => hasFiles(p)
  const bar = (p, files) => ({ path: p, files })
  const surfaceDefs = [
    { label: "kernel-state", path: join(k.dir, "chariox") }, { label: "kernel-history", path: join(k.dir, "history") },
    { label: "kernel-log", path: join(k.dir, "logs") }, { label: "provider-session", path: join(k.dir, "home") },
    { label: "provider-tmp-events", path: join(k.dir, "tmp") }, { label: "kernel-cache", path: join(k.dir, "cache") },
    { label: "session-export", path: exportDir }, { label: "workspace", path: join(k.dir, "work") },
    { label: "browser-frames-ocr", path: join(k.dir, "home", ".claude") },
    { label: "process", kind: "proc", path: "/proc" },
  ]
  const present = surfaceDefs.filter(s => s.kind === "proc" || hasDir(s.path)), absent = surfaceDefs.filter(s => !present.includes(s)).map(s => s.label)
  const real = [
    { label: "real-d1-password", role: "real", value: boat("cat /home/user/fixture-state/f1-received-password.needle") },
    { label: "real-d1-capture", role: "real", value: read(join(needles, "d1-capture.needle")) },
    ...["reveal", "copy", "qr"].map(kind => ({ label: `real-d2-${kind}`, role: "real", value: boat(`cat /home/user/fixture-state/f2-${kind}.needle`) })),
  ].filter(g => g.value)
  const decoys = Object.entries(D).map(([name, value]) => ({ label: `decoy-${name}`, role: "decoy", value }))
  const groups = [...decoys, ...real]
  const { matrix, roots: scannedRoots, ocrAvailable, hitFiles } = await surfaceMatrix({ surfaces: present, groups, ocr: true })
  writeFileSync(join(evidence, "surface-matrix.json"), JSON.stringify({ roots: scannedRoots, absent, matrix, hit_files: Object.fromEntries(Object.entries(hitFiles).map(([g, by]) => [g, Object.fromEntries(Object.entries(by).map(([s, f]) => [s, f.map(x => x.replace(k.dir, "<run>"))]))])) }, null, 2) + "\n")
  // Which live decoy proves which surface: the decoy planted for that surface, not a stand-in.
  const candidates = {
    "kernel-state": ["decoy-chat", "decoy-shell"], "kernel-history": ["decoy-chat"], "kernel-log": ["decoy-alias", "decoy-badid"],
    "provider-session": ["decoy-shell", "decoy-chat"], "provider-tmp-events": ["decoy-shell", "decoy-chat"], "kernel-cache": ["decoy-alias", "decoy-badid"],
    "session-export": ["decoy-card"], "workspace": ["decoy-work"], "browser-frames-ocr": ["decoy-frame"], "process-argv": ["decoy-argv"], "process-environ": ["decoy-environ"],
  }
  const allSurfaces = [...present.filter(s => s.kind !== "proc").map(s => s.label), "process-argv", "process-environ"]
  const decoyOf = Object.fromEntries(allSurfaces.map(s => [s, (candidates[s] ?? []).find(c => matrix[c]?.[s]) ?? (candidates[s] ?? [])[0]]))
  const verdicts = surfaceVerdicts({ surfaces: allSurfaces, matrix, groups, decoyOf, requireOcrFor: ["browser-frames-ocr"] })
  const traceFiles = (() => { try { return execFileSync("find", [k.dir, "-iname", "*trace*"], { encoding: "utf8" }).trim() } catch { return "" } })()
  verdicts.push({ surface: "traces", verdict: traceFiles ? "BLOCKED" : "BLOCKED", reason: traceFiles ? `trace files exist but are not wired into the scan: ${traceFiles.split("\n").length}` : "a drill kernel writes no trace files (nothing named *trace* under its home): there is nothing to scan and no live decoy can land" })
  for (const label of absent) verdicts.push({ surface: label, verdict: "BLOCKED", reason: "this drill kernel keeps no files there (empty or missing directory): nothing to scan, no decoy can land" })
  writeFileSync(join(evidence, "surface-verdicts.json"), JSON.stringify(verdicts, null, 2) + "\n")
  for (const v of verdicts) cell(`never-seen surface ${v.surface}`, v.verdict, v.verdict === "PASS" ? `decoy ${v.decoy} found (${v.decoy_hits} hit(s)); real D1/D2 secrets: 0 hits` : v.verdict === "FAIL" ? `REAL SECRET HIT ${JSON.stringify(v.leaks)} (decoy ${v.decoy} found ${v.decoy_hits})` : v.reason)
  const decoyNotFound = Object.fromEntries(Object.keys(D).map(n => [n, Object.keys(matrix[`decoy-${n}`] ?? {})]))
  cell("decoy -> surfaces where it was found", "INFO", JSON.stringify(decoyNotFound))
  cell("scan coverage", "INFO", `ocr=${ocrAvailable} images=${scannedRoots.reduce((n, r) => n + r.images, 0)} ocr_ok=${scannedRoots.reduce((n, r) => n + r.ocr_ok, 0)} not_scanned=${scannedRoots.reduce((n, r) => n + r.not_scanned, 0)} roots=${scannedRoots.map(r => `${r.surface}:${r.files}`).join(",")}`)
} finally { helper.kill(); await k.stop(); stopFixtures() }
writeFileSync(join(evidence, "cells.json"), JSON.stringify(results, null, 2) + "\n")
