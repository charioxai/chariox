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
import { fixtureState, restartFixtures, scanWithNeedle, stopFixtures } from "./boat.mjs"

const here = dirname(fileURLToPath(import.meta.url))
const scanner = join(here, "../never-seen/never-seen.mjs")
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
// A kernel root that holds no file on this build (e.g. the session history dir, which stays empty) is left out and named in the cell text,
// because the scanner treats an empty root as INVALID.
const roots = work => rootsAll(work).filter(r => r.kind === "proc" || hasFiles(r.path))
const rootsAll = work => [
  { target: "T1", kind: "dir", path: join(k.dir, "chariox") }, { target: "T1", kind: "dir", path: join(k.dir, "history") },
  { target: "T2", kind: "dir", path: join(k.dir, "home") }, { target: "T2", kind: "dir", path: join(k.dir, "tmp") },
  { target: "T3", kind: "dir", path: join(k.dir, "logs") }, { target: "T3", kind: "dir", path: join(k.dir, "cache") },
  { target: "T5", kind: "dir", path: exportDir },
  { target: "T9", kind: "dir", path: work }, { target: "T9", kind: "proc" },
]
const hasFiles = p => { try { return statSync(p).isFile() || readdirSync(p).some(n => hasFiles(join(p, n))) } catch { return false } }
const exportState = async (r, name) => writeFileSync(join(exportDir, name), JSON.stringify(await k.session(r.id)), { mode: 0o600 })
async function scan(name, source, work, control) {
  const rootsFile = join(evidence, `${name}.roots.json`); writeFileSync(rootsFile, JSON.stringify(roots(work)))
  const args = ["--roots", rootsFile, "--ocr", "--out", join(evidence, `${name}.scan.json`), "--run", name, "--build-pin", process.env.D3_BUILD_PIN ?? "main-0a9438b550d"]
  if (control) { writeFileSync(join(needles, `${name}.control`), control, { mode: 0o600 }); args.push("--live-control-file", join(needles, `${name}.control`), "--live-control-targets", "T1,T2") }
  const out = await scanWithNeedle({ source, scanner, args })
  const path = join(evidence, `${name}.scan.json`)
  return { line: out.stdout, report: existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : null }
}
const verdictOf = s => s.report?.verdict === "CLEAN" ? "PASS" : s.report?.verdict === "LEAK" ? "FAIL" : "BLOCKED"
const summary = s => s.report ? `${s.report.verdict} real=${JSON.stringify(s.report.real)} live_control=${JSON.stringify(s.report.control.live)} synthetic_ok=${Object.values(s.report.control.synthetic).every(Boolean)} images=${s.report.ocr.images} ocr_ok=${s.report.ocr.ocr_ok} not_scanned=${s.report.ocr.not_scanned}${s.report.matches.length ? " MATCHES=" + JSON.stringify(s.report.matches.slice(0, 6)) : ""}${s.report.reasons.length ? " reasons=" + s.report.reasons.join("; ") : ""}` : `no scan.json: ${s.line}`

try {
  if (cells.includes("d1")) {
    const work = join(k.dir, "work", "d1"), r = await k.room("d1", work)
    const control = "CONTROL_" + randomBytes(8).toString("hex")
    writeFileSync(join(work, "mk-secret.sh"), `#!/bin/sh\nt=$(openssl rand -hex 24)\nprintf %s "$t" > ${needles}/d1-capture.needle\nprintf %s "$t"\n`, { mode: 0o755 })
    const signup = origins.signup
    const prompt = `Drill task on owned test pages (no real accounts). Work in ${work}. Never print, retype or ask for any secret value; use credential ids only.
1. In your shell run: echo ${control}
2. Sign up at ${signup}/?eye=1&big=1 with a strong generated password: call chariox.vault.generate for the password (site origin ${signup}), load the kernel browser (chariox.load_kernel_browser), type agent@example.invalid in the email field, fill the password and confirm fields with kernel_browser_paste_secret using the generated credential, and submit. Then open the same page again, fill the password field the same way WITHOUT submitting, click the 'Show password' button, and take a snapshot, a screenshot and a find of the password field.
3. Run chariox.vault.use_env with stdin set to the generated credential id and argv ["sh","-c","sha256sum | cut -c1-8"] against origin ${signup}; note the 8 characters it prints.
4. Create a second secret with chariox.vault.capture_command running ${work}/mk-secret.sh (its output is a random token; bind it as command_environment D1_SECRET at origin ${signup}). Then run chariox.vault.use_env with env D1_SECRET=<that id> and argv ["sh","-c","printf %s \\"$D1_SECRET\\" | sha256sum | cut -c1-8; echo value=$D1_SECRET"] and note exactly what the tool result shows.
5. Try chariox.vault.generate with a command_environment use binding (a random secret for a command, not a site password); say in one line whether it was accepted or refused, with the refusal text.
Write a plain-text report to ${work}/report.txt with: the control string, the 8 characters from step 3, what step 4's result shows, what the password field looked like in the screenshot after Show password (describe it; do not guess the value), the step 5 answer, and every Chariox tool you used. Final answer: DONE.`
    const cards = await k.ask(r, prompt, { timeoutMs: 900000 })
    await exportState(r, "d1-session.json")
    const report = read(join(work, "report.txt")); writeFileSync(join(evidence, "d1-report.txt"), report, { mode: 0o600 })
    const st = await fixtureState(origins.signup), received = st.accounts.at(-1)
    cell("D1-1 sign-up with a generated password", received ? "PASS" : "FAIL", received ? `fixture: account created (password hash ${received.password_sha8}); cards=${cards.length}` : `fixture shows no account; log=${JSON.stringify(st.log.slice(-2))}; cards=${JSON.stringify(cards)}`)
    cell("D1-1 entry equals the typed value (hash)", received && report.includes(received.password_sha8) ? "PASS" : "FAIL", `fixture hash ${received?.password_sha8 ?? "none"}; in the agent's report: ${received ? report.includes(received.password_sha8) : "n/a"}`)
    const captured = read(join(needles, "d1-capture.needle"))
    cell("D1-3 capture_command + use_env, value masked in the result", captured && report.includes(sha8(captured)) && !report.includes(captured) ? "PASS" : "FAIL", captured ? `hash in report: ${report.includes(sha8(captured))}; raw value in report: ${report.includes(captured)}` : "the script never wrote its needle (not run, or the write was blocked)")
    const s1 = await scan("d1-signup-password", { boat: "/home/user/fixture-state/f1-received-password.needle" }, work, control)
    cell("D1-4 never-seen: the generated sign-up password (T1 T2 T3 T5 T9, OCR)", verdictOf(s1), summary(s1))
    if (captured) { const s2 = await scan("d1-capture-token", { file: join(needles, "d1-capture.needle") }, work, control); cell("D1-4 never-seen: the capture_command token", verdictOf(s2), summary(s2)) }
    else cell("D1-4 never-seen: the capture_command token", "BLOCKED", "no needle (script did not run)")
    cell("D1-3b generate with a command binding (row 556)", !report ? "BLOCKED" : /refus|not|cannot|only/i.test(report.split("step 5")[1] ?? report) ? "FAIL" : "PASS", "see d1-report.txt: step 5 answer")
  }
  if (cells.includes("d2")) {
    const work = join(k.dir, "work", "d2"), r = await k.room("d2", work), reveal = origins.reveal
    const control = "CONTROL_" + randomBytes(8).toString("hex")
    const prompt = `Drill task on an owned test page (no real accounts). Work in ${work}. Never print, retype or ask for any token value; use credential ids only.
1. In your shell run: echo ${control}
2. Load the kernel browser and open ${reveal}/ . The page shows a token once in three ways (Reveal button, Copy button, QR).
3. Click 'Reveal token', then capture the revealed value straight into the Vault with chariox.vault.capture_browser (read the node; use reveal_node_ref if needed), use_binding command_environment FIXTURE_TOKEN at origin ${reveal}.
4. Click 'Copy token', then capture it with chariox.vault.capture_browser reading the clipboard, same binding style, a second credential.
5. Capture the QR code the same way (read the QR), a third credential.
6. Verify each with ONE chariox.vault.use_env call each: argv ["curl","-s","-o","/dev/null","-w","%{http_code}","-X","POST","-H","Authorization: Bearer $FIXTURE_TOKEN","${reveal}/verify?kind=reveal"] (use kind=copy and kind=qr for the others; run it through sh -c so $FIXTURE_TOKEN expands). Report only HTTP codes and credential ids.
For any step that fails, say which tool refused and its exact message. Write a plain-text report to ${work}/report.txt with the control string, each step's outcome (credential id, HTTP code), the size of the largest browser tool reply you saw (rough), and every Chariox tool you used. Final answer: DONE.`
    const cards = await k.ask(r, prompt, { timeoutMs: 1200000 })
    await exportState(r, "d2-session.json")
    const report = read(join(work, "report.txt")); writeFileSync(join(evidence, "d2-report.txt"), report, { mode: 0o600 })
    const st = await fixtureState(reveal), oks = Object.fromEntries(["reveal", "copy", "qr"].map(kind => [kind, st.log.some(e => e.kind === "verify" && e.block === kind && e.ok)]))
    for (const [kind, ok] of Object.entries(oks)) cell(`D2-4 ${kind}: captured into the Vault and verified at the fixture`, ok ? "PASS" : "FAIL", `fixture server log: verify ${kind} ok=${ok}; cards=${cards.length}`)
    for (const [kind, file] of [["reveal", "f2-reveal.needle"], ["copy", "f2-copy.needle"], ["qr", "f2-qr.needle"]]) {
      const s = await scan(`d2-${kind}`, { boat: `/home/user/fixture-state/${file}` }, work, control)
      cell(`D2-2 never-seen: the ${kind} token (T1 T2 T3 T5 T9, OCR)`, verdictOf(s), summary(s))
    }
  }
} finally { await k.stop(); stopFixtures() }
writeFileSync(join(evidence, "cells.json"), JSON.stringify(results, null, 2) + "\n")
