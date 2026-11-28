#!/usr/bin/env node
// Row 558 flake: D1 sign-up (generate + paste_secret + Show password) and then a QR capture, several times in the SAME kernel browser.
//   node qr-flake.mjs --evidence <dir under /root/.chariox/dev/> [--rounds 4] [--pause 40] [--model ...]
// Env as run-live.mjs; D3_KERNEL_SCRIPT (optional) = a copy of the kernel-browser docker dir whose controller logs the real exception
// (D3_DEBUG_FILE), because the kernel's own error label hides it ("host browser operation failed"). Prints one line per round.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { kernelBrowserScript } from "./paths.mjs"
import { startDrillKernel } from "./drill-kernel.mjs"
import { restartFixtures, stopFixtures, fixtureState } from "./boat.mjs"

const flags = new Map(); for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
const evidence = flags.get("--evidence")
if (!evidence?.startsWith("/root/.chariox/dev/")) throw new Error("--evidence must be under /root/.chariox/dev/")
const rounds = Number(flags.get("--rounds") ?? 4), pause = Number(flags.get("--pause") ?? 0)
mkdirSync(evidence, { recursive: true, mode: 0o700 })
const browserScript = kernelBrowserScript(process.env.D3_KERNEL_SCRIPT)
const origins = restartFixtures()
const debugFile = join(evidence, "controller-errors.jsonl")
const extraEnv = { D3_DEBUG_FILE: debugFile, ...(browserScript ? { CHARIOX_KERNEL_BROWSER_SCRIPT: browserScript } : {}) }
const k = await startDrillKernel({ label: "d3qr", binary: process.env.D3_KERNEL_BIN, stateRoot: "/root/.chariox/dev/orch15-vaultd3prep/state", evidence, tokenFile: "/root/.claude-lanes/oauth-token.chariox", model: flags.get("--model") ?? "claude-haiku-5-5", extraEnv })
try {
  const work = join(k.dir, "work", "qr"), r = await k.room("qr", work)
  const prompt = `Drill task on owned test pages (no real accounts). Work in ${work}. Never print or retype any secret value; use credential ids only. Do these steps ${rounds} times in a row (round 1..${rounds}), all in the SAME kernel browser:
A. Open ${origins.signup}/?eye=1&big=1 . Use chariox.vault.generate once (round 1 only; reuse that credential later) for site origin ${origins.signup}. Fill the password and confirm fields with kernel_browser_paste_secret, type agent@example.invalid in the email field, submit. Then open the page again, fill the password field the same way, click 'Show password', and take a snapshot and a screenshot.
B. Open ${origins.reveal}/ in a NEW tab, find the QR code element on it,${pause ? ` then wait ${pause} seconds by running this exact shell command: node -e "setTimeout(()=>{},${pause * 1000})" (a standalone sleep is blocked in your shell),` : ""} and capture it with chariox.vault.capture_browser (read=qr) with use_binding command_environment FIXTURE_TOKEN at origin ${origins.reveal}.
If B fails, write down the exact tool name and complete error text, then do NOT retry: go on with the next round. Write ${work}/report.txt with one line per round: round N, A ok/failed, B ok/failed + credential id or the exact error text and the node_ref you used. Final answer: DONE.`
  const cards = await k.ask(r, prompt, { timeoutMs: 1800000 })
  const report = existsSync(join(work, "report.txt")) ? readFileSync(join(work, "report.txt"), "utf8") : ""
  writeFileSync(join(evidence, "report.txt"), report, { mode: 0o600 })
  console.log("REPORT\n" + report)
  console.log("CONTROLLER ERRORS", existsSync(debugFile) ? readFileSync(debugFile, "utf8").split("\n").filter(Boolean).length : 0)
} finally { await k.stop(); stopFixtures() }
