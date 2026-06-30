#!/usr/bin/env node
import { spawn } from "node:child_process"
import { readFile } from "node:fs/promises"
import path from "node:path"

const FOCUSED_TESTS = [
  "dist/transcript-collapsed-blob.test.js",
  "dist/transcript-display.test.js",
  "dist/queued-prompt-transcript.test.js",
  "dist/queued-prompt-strip-state.test.js",
  "dist/agent-interaction-strip-controller.test.js",
  "dist/prompt-keydown-controller.test.js",
  "dist/footer-summary-compact.test.js",
  "dist/split-pane-footer.test.js",
  "dist/session-chrome-render-controller.test.js",
  "dist/session-chrome-state.test.js",
  "dist/prompt-chrome-projection-controller.test.js",
  "dist/waiting-room-session-rows.test.js",
  "dist/waiting-room-rows.test.js",
  "dist/waiting-room-menu-row.test.js",
  "dist/history-loading-render-controller.test.js",
  "dist/cli-loading-state-controller.test.js",
  "dist/transcript-history-autoload-controller.test.js",
  "dist/agent-pane-state.test.js",
  "dist/cli-automation-snapshot.test.js",
  "dist/cli-automation-handler.test.js",
]

const REPO_ROOT = path.resolve(import.meta.dirname, "../../..")
const CLI_ROOT = path.resolve(import.meta.dirname, "..")
const PLAN_PATH = path.join(REPO_ROOT, "docs/TUI_WEB_TERMINAL_UX_PARITY_PLAN.html")

async function main() {
  const startedAt = new Date()
  const steps = []

  await runStep(steps, "focused-tests", process.execPath, ["--test", ...FOCUSED_TESTS], { cwd: CLI_ROOT })
  await runStep(steps, "visual-session-syntax", process.execPath, ["--check", "scripts/live-tui-web-parity-visual-session.mjs"], { cwd: CLI_ROOT })
  await runStep(steps, "visual-control-syntax", process.execPath, ["--check", "scripts/tui-web-parity-visual-control.mjs"], { cwd: CLI_ROOT })
  await runStep(steps, "agent-footer-status-drill", process.execPath, ["scripts/agent-footer-status-drill.mjs"], { cwd: CLI_ROOT })
  await runStep(steps, "history-outline-tui-drill", process.execPath, ["scripts/history-outline-tui-drill.mjs"], { cwd: CLI_ROOT })
  await verifyPlan()
  steps.push({ name: "plan-gap-check", status: "passed" })

  const completedAt = new Date()
  console.log(JSON.stringify({
    schema: "arroba.tui_web_terminal_parity_drill.v1",
    status: "passed",
    startedAt: startedAt.toISOString(),
    completedAt: completedAt.toISOString(),
    durationMs: completedAt.getTime() - startedAt.getTime(),
    steps,
  }, null, 2))
}

async function runStep(steps, name, command, args, options) {
  const startedAt = Date.now()
  const child = spawn(command, args, {
    ...options,
    stdio: "inherit",
  })
  const result = await new Promise((resolve) => {
    child.on("exit", (code, signal) => resolve({ code, signal }))
    child.on("error", (error) => resolve({ code: 1, signal: null, error }))
  })
  const durationMs = Date.now() - startedAt
  if (result.error) {
    steps.push({ name, status: "failed", durationMs, error: String(result.error.message ?? result.error) })
    throw result.error
  }
  if (result.signal || result.code !== 0) {
    const message = `${name} failed with code ${result.code ?? "null"} signal ${result.signal ?? "none"}`
    steps.push({ name, status: "failed", durationMs, exitCode: result.code, signal: result.signal })
    throw new Error(message)
  }
  steps.push({ name, status: "passed", durationMs })
}

async function verifyPlan() {
  const html = await readFile(PLAN_PATH, "utf8")
  const requiredMarkers = [
    "Live PTY Drill Update - 2026-06-30",
    "Dedicated queued-prompt strip",
    "Queue scrollback cleanup",
    "Queue keyboard selection",
    "Queue notice suppression",
    "Compact screen footer summary",
    "Collapsed blob presentation",
    "broader live-drill assertions",
  ]
  const missing = requiredMarkers.filter((marker) => !html.includes(marker))
  if (missing.length > 0) {
    throw new Error(`parity plan is missing current evidence markers: ${missing.join(", ")}`)
  }

  const retiredMarkers = [
    "Mostly aligned",
    "Tests to Add or Update",
  ]
  const found = retiredMarkers.filter((marker) => html.includes(marker))
  if (found.length > 0) {
    throw new Error(`parity plan still contains stale markers: ${found.join(", ")}`)
  }
}

main().catch((error) => {
  console.error(`[tui-web-terminal-parity-drill] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
