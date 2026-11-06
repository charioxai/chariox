#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 A6/A7 (TUI): drives the built CLI in a private tmux server against the fixture kernel feed
// and records text and ANSI frames. Fixture-driven: a PASS here is PASS-fixture, not a live PASS.
// Usage: node sub-agents-tui-drill.mjs <evidence-dir> [cols...]   (needs apps/cli/dist built, tmux, bun)
import assert from "node:assert/strict"
import { execFileSync, spawn } from "node:child_process"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const evidence = path.resolve(process.argv[2] ?? "")
const widths = process.argv.slice(3).map(Number)
assert.ok(process.argv[2], "usage: sub-agents-tui-drill.mjs <evidence-dir> [cols...]")
mkdirSync(evidence, { recursive: true })
const sleep = ms => new Promise(done => setTimeout(done, ms))
const tmux = (...args) => execFileSync("tmux", ["-L", "subtui-drill", ...args], { encoding: "utf8" })

const node = (id, parent, state, n, needs_user = false) => ({ agent_id: id, parent_agent_id: parent, kernel_id: null, alias: id, provider: "claude", model: null, depth: 1, state, outcome: null, deadline_ms: null, created_at_ms: n, needs_user })
const tree = (rev, nodes) => ({ root: "room-agent", rev, totals: { live: nodes.filter(x => ["starting", "running", "waiting", "unreachable"].includes(x.state)).length, running: nodes.filter(x => x.state === "running").length, needs_user: nodes.filter(x => x.needs_user).length }, nodes })
const R = "room-agent"
const trees = {
  none: tree(1, []),
  one: tree(2, [node("researcher", R, "running", 1)]),
  six: tree(3, [node("researcher", R, "running", 1), node("reader", "researcher", "running", 2), node("parser", "reader", "running", 3), node("writer", R, "running", 4), node("reviewer", "writer", "starting", 5), node("tester", R, "running", 6)]),
  mixed: tree(4, [node("researcher", R, "running", 1), node("reader", "researcher", "waiting", 2, true), node("parser", "reader", "waiting", 3), node("writer", R, "running", 4), node("reviewer", "writer", "ended", 5), node("tester", R, "idle", 6)]),
  readerDone: tree(5, [node("researcher", R, "running", 1), node("reader", "researcher", "ended", 2), node("parser", "reader", "waiting", 3), node("writer", R, "running", 4), node("reviewer", "writer", "ended", 5), node("tester", R, "idle", 6)]),
  lastLive: tree(6, [node("researcher", R, "ended", 1), node("reader", "researcher", "ended", 2), node("parser", "reader", "idle", 3), node("writer", R, "running", 4), node("reviewer", "writer", "ended", 5), node("tester", R, "idle", 6)]),
  allDone: tree(7, [node("researcher", R, "ended", 1), node("reader", "researcher", "ended", 2), node("parser", "reader", "idle", 3), node("writer", R, "ended", 4), node("reviewer", "writer", "ended", 5), node("tester", R, "idle", 6)]),
  dropped: tree(8, []),
}

async function run(cols, port) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "subtui-drill-"))
  const home = mkdtempSync(path.join(os.tmpdir(), "subtui-home-"))
  const out = path.join(evidence, `cols${cols}`)
  mkdirSync(out, { recursive: true })
  const kernel = spawn("node", [path.join(here, "sub-agents-fixture-kernel.mjs"), String(port), dir], { stdio: "ignore" })
  const session = `subtui-${cols}`
  const setTree = name => writeFileSync(path.join(dir, "tree.json"), JSON.stringify(trees[name]))
  const screen = () => tmux("capture-pane", "-p", "-t", session).split("\n").map(line => line.trimEnd())
  const waitFor = async (test, label) => {
    for (let attempt = 0; attempt < 60; attempt += 1) {
      const lines = screen()
      if (test(lines.join("\n"))) return lines.join("\n")
      await sleep(250)
    }
    throw new Error(`timed out: ${label}\n${screen().join("\n")}`)
  }
  let step = 0
  const shot = async (name, test) => {
    const text = await waitFor(test, name)
    step += 1
    const file = `${String(step).padStart(2, "0")}-${name}`
    writeFileSync(path.join(out, `${file}.txt`), `${text}\n`)
    writeFileSync(path.join(out, `${file}.ansi`), tmux("capture-pane", "-p", "-e", "-t", session))
    return text
  }
  const keys = (...names) => tmux("send-keys", "-t", session, ...names)
  const results = []
  const check = (name, ok, detail = "") => { results.push({ name, ok, detail }); assert.ok(ok, `${name} ${detail}`) }
  try {
    await sleep(500)
    tmux("new-session", "-d", "-s", session, "-x", String(cols), "-y", "34", `env CHARIOX_HOME=${home} bun ${path.join(here, "..", "dist", "index.js")} --kernel-url ws://127.0.0.1:${port} --session room1`)
    await shot("attached-no-sub-agents", text => /Write your next prompt here/.test(text))
    check("no stripe without sub-agents", !/Agents \d/.test(screen().join("\n")))

    setTree("one")
    const one = await shot("one-agent", text => /1 agent running/.test(text))
    check("one agent reads singular", /Agents 1 agent running/.test(one))

    setTree("six")
    const six = await shot("six-agents-three-depths", text => /6 agents running/.test(text))
    check("six agents plain count", /Agents 6 agents running/.test(six))

    keys("F7")
    const card = await shot("card-six", text => /researcher/.test(text) && /parser/.test(text))
    const rows = card.split("\n")
    const col = name => rows.find(line => line.includes(name))?.indexOf(name) ?? -1
    check("card nests sub-sub-agents at every depth", col("researcher") < col("reader") && col("reader") < col("parser") && col("writer") < col("reviewer"), `${col("researcher")} ${col("reader")} ${col("parser")}`)
    check("card words", rows.filter(line => /\brunning\b/.test(line)).length >= 6)
    check("card has no title, hint or error text", !/Esc|error|failed|Sub-agents/i.test(card.split("\n").slice(-34).filter(line => line.includes("researcher") || line.includes("reader")).join("\n")))

    keys("Left")
    const collapsed = await shot("card-collapsed-researcher", text => !/\breader\b/.test(text.replace("researcher", "")))
    check("Left collapses the row's descendants", !/\bparser\b/.test(collapsed))
    keys("Right")
    await waitFor(text => /parser/.test(text), "expanded again")

    keys("Down")
    keys("Enter")
    const view = await shot("view-opened-focus-reader", text => /Sub-agents 6/.test(text))
    check("a row opens the closed view", /Sub-agents 6/.test(view) && !/\bparser\s+running\s*\n.*\n.*Esc/.test(view))
    check("the prompt stays on the room agent", /Write your next prompt here/.test(view))

    keys("C-Down")
    const moved = await shot("view-focus-moved", () => true)
    await sleep(300)
    keys("hello")
    const typed = await shot("prompt-still-types", text => /hello/.test(text))
    check("typing still reaches the room agent's prompt", /hello/.test(typed))
    keys("C-u")

    setTree("mixed")
    const mixed = await shot("mixed-needs-you-waiting-done", text => /1 agent needs you · 2 running · 1 waiting · 2 done/.test(text))
    check("needs you leads", /Agents 1 agent needs you · 2 running · 1 waiting · 2 done/.test(mixed))
    check("panels say waiting while a decision is held", /reader\s+waiting/.test(mixed) && !/reader\s+needs you/.test(mixed))
    check("no decision card in the view", !/\[|Approve|Deny|Allow/.test(mixed.split("\n").filter(line => /reader|parser|writer/.test(line)).join("\n")))

    keys("F7")
    const card2 = await shot("card-mixed", text => /reader\s+needs you/.test(text))
    check("card: per-row needs you from the node flag only", /reader\s+needs you/.test(card2) && /parser\s+waiting/.test(card2), "")
    keys("Escape")
    await waitFor(text => !/reader\s+needs you/.test(text), "card closed")

    setTree("readerDone")
    const done = await shot("node-finished-while-focused", text => /reader\s+done/.test(text))
    check("a finished node keeps its panel and the view stays", /Sub-agents 6/.test(done) && /reader\s+done/.test(done))

    setTree("lastLive")
    await shot("last-live-writer", text => /1 agent running · 5 done/.test(text))
    setTree("allDone")
    const ended = await shot("last-sub-agent-ended-view-closed", text => /6 agents done/.test(text) && !/Sub-agents 6/.test(text))
    check("the view closes with the last live sub-agent; the room stays on its agents, no blank view", /Write your next prompt here/.test(ended) && !/Sub-agents \d/.test(ended))
    keys("F7")
    await shot("card-all-done", text => /researcher/.test(text))
    keys("Enter")
    const afterRow = await shot("row-with-none-live-only-closes-card", text => !/researcher\s+done/.test(text) && /6 agents done/.test(text))
    check("a row with none live opens no view", !/Sub-agents \d/.test(afterRow))

    setTree("dropped")
    const gone = await shot("parent-task-ended-stripe-gone", text => !/Agents \d/.test(text))
    check("no sub-agents, no stripe", !/Agents \d/.test(gone))
    writeFileSync(path.join(out, "results.json"), JSON.stringify({ cols, checks: results }, null, 2))
    return results
  } finally {
    writeFileSync(path.join(out, "results.json"), JSON.stringify({ cols, checks: results }, null, 2))
    try { tmux("kill-session", "-t", session) } catch {}
    kernel.kill("SIGTERM")
    rmSync(dir, { recursive: true, force: true }); rmSync(home, { recursive: true, force: true })
  }
}

let port = 47400 + Math.floor(Math.random() * 400)
try {
  for (const cols of widths) {
    const results = await run(cols, port++)
    console.log(JSON.stringify({ cols, passed: results.length, checks: results.map(r => r.name) }))
  }
} finally {
  try { tmux("kill-server") } catch {}
}
