#!/usr/bin/env bun
// MP-08 / MP-10 / MP-11: run against two existing normal-login Claude profiles.
// Provider turns and account changes enter through the real TUI prompt area.
import assert from "node:assert/strict"
import { createHash, randomUUID } from "node:crypto"
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"

const usage = `MP-08 / MP-10 / MP-11 — two existing Claude accounts, real TUI
bun apps/cli/scripts/live-claude-two-accounts.mjs
  --kernel-url <url> --operator-home <absolute CHARIOX_HOME>
  --profiles <profile-A,profile-B> --normal-login-confirmed
  --cli <built apps/cli/dist/index.js> --workspace <trusted directory>
  --tools <directory with playwright and @xterm/xterm installed>
  --chromium <executable> --source <kernel commit> --evidence <outside repo>
  --scratch <lane-owned disk temp directory, required with --execute>
  [--model haiku] [--dpr 1|2] [--timeout-ms 300000] [--execute]

Default: scoped, read-only auth/usage preflight. --execute runs A/B in parallel
for claude-p and claude-headless, then switches A to B through /account and
checks the next turn. Neither mode signs in/out or copies credentials.
The operator must supply two different accounts with usable NORMAL logins;
public auth status does not identify the credential backing mechanism.
Workspace startup approvals must already be satisfied. Any interaction fails
the run for operator inspection. Only sessions created by this script are deleted.
Run provider turns only at the scheduled validation time.`

if (process.argv.includes("--help")) { console.log(usage); process.exit(0) }
function option(name, fallback = null) {
  const index = process.argv.indexOf(`--${name}`)
  return index < 0 ? fallback : process.argv[index + 1]
}
const required = name => { const value = option(name); assert(value, `missing --${name}`); return value }
const profiles = required("profiles").split(",")
assert.equal(profiles.length, 2)
assert(profiles.every(value => /^[a-zA-Z0-9._-]+$/.test(value)), "use exact public profile IDs")
assert.notEqual(profiles[0], profiles[1])
assert(process.argv.includes("--normal-login-confirmed"), "operator must confirm normal logins")
const execute = process.argv.includes("--execute")
const kernelUrl = required("kernel-url")
const home = await realpath(required("operator-home"))
const workspace = await realpath(required("workspace"))
const cli = await realpath(required("cli"))
const evidence = path.resolve(required("evidence"))
const source = required("source")
const model = option("model", "haiku")
const dpr = Number(option("dpr", "2"))
const timeout = Number(option("timeout-ms", "300000"))
assert([1, 2].includes(dpr))
assert(Number.isFinite(timeout) && timeout >= 1000 && timeout <= 900000)
const repo = path.resolve(path.dirname(cli), "../../..")
assert(![repo, workspace, home].some(root => evidence === root || evidence.startsWith(root + path.sep)), "evidence must be outside source, workspace and runtime state")
if (execute) assert(typeof Bun !== "undefined", "real TUI execution requires Bun")
await mkdir(evidence, { recursive: true })
const { LocalIpcClient } = await import(pathToFileURL(path.join(path.dirname(cli), "ipc.js")))
const client = new LocalIpcClient(kernelUrl)
const runTag = `claude-accounts-${randomUUID()}`
const cells = []
const result = { items: ["MP-08", "MP-10", "MP-11"], source, model, dpr, mode: execute ? "real-tui" : "read-only-preflight", started: new Date().toISOString(), profiles, checks: [] }
let browser, frontend, scratch
const payload = (response, key) => { assert(response?.[key], `expected ${key}; received ${Object.keys(response ?? {}).join(",")}`); return response[key] }
const request = async (key, body = {}) => payload(await client.send({ [key]: body }), ({ ListSessions: "SessionsListed", GetSessionState: "SessionState", ListProviderAccountProfiles: "ProviderAccountProfilesListed", GetProviderAuthStatus: "ProviderAuthStatus", GetSessionHistoryOutline: "SessionHistoryOutline", GetSessionHistoryBlobContent: "SessionHistoryBlobContent", GetProviderRun: "ProviderRun", DeleteSession: "SessionDeleted" })[key])
const hash = value => createHash("sha256").update(value).digest("hex")

async function scopedAccounts(stage) {
  const listed = (await request("ListProviderAccountProfiles", { provider: "claude" })).profiles
  const observed = []
  for (const profileId of profiles) {
    const profile = listed.find(value => value.profile_id === profileId)
    assert(profile, `missing registered profile ${profileId}`)
    const status = (await request("GetProviderAuthStatus", { provider: "claude", account_profile: profileId })).status
    assert.equal(status.account_profile, profileId, "unscoped auth response")
    assert.equal(status.auth_state, "authenticated", `profile ${profileId} is not signed in`)
    assert(status.identity_summary?.trim(), `missing public account identity for ${profileId}`)
    assert.equal(profile.usage.profile_id, profileId, "usage belongs to another profile")
    observed.push({ profile_id: profileId, identity_sha256: hash(status.identity_summary.trim()), auth_state: status.auth_state, plan: status.plan, version: status.detected_version, usage: { provider: profile.usage.provider, profile_id: profile.usage.profile_id, availability: profile.usage.availability, source: profile.usage.source, observed_at_ms: profile.usage.observed_at_ms, meters: profile.usage.meters } })
  }
  assert.notEqual(observed[0].identity_sha256, observed[1].identity_sha256, "both profiles identify the same account")
  result.checks.push({ stage, accounts: observed })
  return observed
}

async function state(cell) {
  const session = (await request("GetSessionState", { session_id: cell.sessionId })).session
  assert.equal(session.alias, cell.alias)
  const agent = session.agents.find(value => value.id === cell.agentId)
  assert(agent)
  assert.equal(agent.provider, cell.provider)
  assert.equal(agent.account_profile, cell.profileId, "prompt area selected the wrong account")
  assert(!(session.active_interactions?.length), "interaction requires operator inspection; no automatic approval")
  return { session, agent }
}

async function input(cell, text) {
  await cell.page.locator(".xterm-helper-textarea").focus()
  await cell.page.keyboard.type(text, { delay: 8 })
  await cell.page.keyboard.press("Enter")
}

async function capture(cell, step) {
  await writeFile(path.join(evidence, `${cell.key}-${step}-console.json`), JSON.stringify(cell.console, null, 2))
  await cell.page.screenshot({ path: path.join(evidence, `${cell.key}-${step}.png`) })
  await writeFile(path.join(evidence, `${cell.key}-${step}.txt`), await cell.page.evaluate(() => Array.from({ length: term.rows }, (_, i) => term.buffer.active.getLine(term.buffer.active.viewportY + i)?.translateToString(true) ?? "").join("\n")))
}

async function startCell(provider, profileId, index) {
  const cell = { key: `${provider}-${index}`, alias: `${runTag}-${provider}-${index}`, provider, profileId, sessionId: null, agentId: null, tail: "", sockets: new Set(), console: [] }
  cells.push(cell)
  cell.process = Bun.spawn(["bun", cli, "--kernel-url", kernelUrl, "--create-session", "--alias", cell.alias, "--workspace", workspace, "--worktree", workspace, "--provider", provider, "--account-profile", profileId, "--model", model], {
    cwd: workspace, env: { ...process.env, CHARIOX_HOME: home, TMPDIR: scratch, NODE_OPTIONS: "--max-old-space-size=4096", TERM: "xterm-256color" },
    terminal: { cols: 110, rows: 38, data(_terminal, chunk) { const value = new TextDecoder().decode(chunk); cell.tail = (cell.tail + value).slice(-2_000_000); for (const socket of cell.sockets) socket.send(value) } },
  })
  cell.page = await browser.newPage({ viewport: { width: 1250, height: 800 }, deviceScaleFactor: dpr })
  cell.page.on("console", message => { if (["warning", "error"].includes(message.type()) && cell.console.length < 100) cell.console.push({ type: message.type(), text: message.text() }) })
  cell.page.on("pageerror", error => { if (cell.console.length < 100) cell.console.push({ type: "pageerror", text: error.message }) })
  await cell.page.goto(`http://127.0.0.1:${frontend.port}/?cell=${cell.key}`)
  await cell.page.waitForFunction(() => window.term && window.ws?.readyState === 1)
  const deadline = Date.now() + 60000
  while (Date.now() < deadline) {
    const session = (await request("ListSessions")).sessions.find(value => value.alias === cell.alias)
    if (session) {
      cell.sessionId = session.id
      const attached = (await request("GetSessionState", { session_id: session.id })).session
      cell.agentId = attached.focused_agent_id ?? attached.agents[0]?.id
      break
    }
    if (cell.process.exitCode !== null) throw new Error(`TUI ${cell.key} exited before attachment`)
    await sleep(250)
  }
  assert(cell.sessionId && cell.agentId, `TUI ${cell.key} did not create its session`)
  await sleep(3000)
  await capture(cell, "attached")
  await state(cell)
  await input(cell, "/permissions required")
  await sleep(1000)
  await capture(cell, "permissions-required")
  return cell
}

async function completed(cell, marker, forbidden) {
  const history = (await request("GetSessionHistoryOutline", { session_id: cell.sessionId, agent_ids: [cell.agentId], latest_prompt_count: 4, cursor: null })).agents
  const turn = history.find(agent => agent.agent_id === cell.agentId)?.turns.find(turn => turn.user_prompt?.entry?.text.includes(marker))
  if (!turn) return null
  for (const blob of turn.blobs.filter(blob => ["provider_output", "provider_error"].includes(blob.kind))) {
    turn.entries.push(...(await request("GetSessionHistoryBlobContent", { session_id: cell.sessionId, agent_id: cell.agentId, blob_id: blob.blob_id })).entries)
  }
  const entries = [...turn.entries, ...(turn.summary ? [turn.summary] : [])].map(value => value.entry)
  assert(turn.lifecycle !== "cancelled", "turn cancelled")
  assert(!entries.some(value => value.kind === "provider_error"), "provider error in account test turn")
  if (turn.lifecycle !== "completed") return null
  const output = entries.filter(value => value.kind === "provider_output")
  const text = output.map(value => value.text).join("")
  if (!text.includes(marker)) return null
  assert(forbidden.every(value => !text.includes(value)), "other account's marker crossed into this turn")
  const runIds = [...new Set(output.map(value => value.provider_run_id).filter(Boolean))]
  assert(runIds.length, "missing provider-run attribution")
  for (const runId of runIds) {
    const run = (await request("GetProviderRun", { provider_run_id: runId })).provider_run
    assert.equal(run.session_id, cell.sessionId)
    assert.equal(run.agent_instance_id, cell.agentId)
    assert.equal(run.account_profile, cell.profileId)
    assert.equal(run.provider, cell.provider)
  }
  const visible = await cell.page.evaluate(value => Array.from({ length: term.rows }, (_, i) => term.buffer.active.getLine(term.buffer.active.viewportY + i)?.translateToString(true) ?? "").join("\n").includes(value), marker)
  if (!visible) return null
  return { key: cell.key, profile_id: cell.profileId, session_id: cell.sessionId, agent_id: cell.agentId, run_ids: runIds, marker, turn_id: turn.turn_id }
}

async function turns(pair, stage) {
  const markers = pair.map((cell, index) => `ACCOUNT_CHECK_${runTag.replaceAll("-", "_")}_${stage}_${index}`)
  await Promise.all(pair.map((cell, index) => input(cell, `Reply exactly ${markers[index]}. Do not use tools.`)))
  const deadline = Date.now() + timeout
  let concurrent = pair.length === 1
  while (Date.now() < deadline) {
    const states = await Promise.all(pair.map(state))
    const active = states.map(({ session }, index) => {
      const activities = session.agent_activity ?? {}
      const activity = Array.isArray(activities) ? activities.find(value => value.agent_id === pair[index].agentId) : activities[pair[index].agentId]
      return activity?.busy && activity.active_turn?.status === "running" && activity.active_turn.provider_run_id
    })
    if (active.every(Boolean)) {
      const runs = await Promise.all(active.map(providerRunId => request("GetProviderRun", { provider_run_id: providerRunId })))
      for (let index = 0; index < runs.length; index++) assert.equal(runs[index].provider_run.account_profile, pair[index].profileId)
      if (runs.every(value => String(value.provider_run.state).toLowerCase() === "running")) concurrent = true
    }
    const receipts = await Promise.all(pair.map((cell, index) => completed(cell, markers[index], markers.filter((_, other) => other !== index))))
    if (receipts.every(Boolean)) {
      assert(concurrent, "parallel active prompts were not observed")
      await Promise.all(pair.map(cell => capture(cell, stage)))
      result.checks.push({ stage, concurrent, receipts })
      return
    }
    await sleep(250)
  }
  await Promise.all(pair.map(cell => capture(cell, `${stage}-timeout`)))
  throw new Error(`timed out waiting for visible, attributed ${stage} output`)
}

try {
  const before = await scopedAccounts("before")
  if (execute) {
    const pool = await realpath(required("scratch"))
    assert(pool !== "/tmp" && !pool.startsWith("/tmp/") && ![repo, workspace, home].some(root => pool === root || pool.startsWith(root + path.sep)), "use lane-owned disk scratch outside source and runtime state")
    scratch = await mkdtemp(path.join(pool, "claude-accounts-"))
    process.env.TMPDIR = scratch
    const tools = await realpath(required("tools"))
    const { chromium } = await import(pathToFileURL(path.join(tools, "node_modules/playwright/index.mjs")))
    browser = await chromium.launch({ executablePath: required("chromium"), headless: true, args: ["--no-sandbox"] })
    frontend = Bun.serve({ hostname: "127.0.0.1", port: 0,
      fetch(request, server) {
        const url = new URL(request.url)
        if (url.pathname === "/terminal") {
          const cell = cells.find(cell => cell.key === url.searchParams.get("cell"))
          if (cell && server.upgrade(request, { data: cell })) return
          return new Response("unknown cell", { status: 404 })
        }
        if (url.pathname === "/xterm.js") return new Response(Bun.file(path.join(tools, "node_modules/@xterm/xterm/lib/xterm.js")))
        if (url.pathname === "/xterm.css") return new Response(Bun.file(path.join(tools, "node_modules/@xterm/xterm/css/xterm.css")))
        return new Response(`<link rel="stylesheet" href="/xterm.css"><style>body{background:#141414;margin:16px}</style><div id="terminal"></div><script src="/xterm.js"></script><script>window.term=new Terminal({cols:110,rows:38,fontSize:16,scrollback:2000});term.open(document.querySelector('#terminal'));term.focus();window.ws=new WebSocket('ws://'+location.host+'/terminal'+location.search);ws.onmessage=e=>term.write(e.data);term.onData(data=>ws.send(data));</script>`, { headers: { "content-type": "text/html" } })
      },
      websocket: { open(socket) { socket.data.sockets.add(socket); socket.send(socket.data.tail) }, close(socket) { socket.data.sockets.delete(socket) }, message(socket, data) { socket.data.process.terminal.write(String(data)) } },
    })
    for (const provider of ["claude-p", "claude-headless"]) {
      const pair = await Promise.all(profiles.map((profile, index) => startCell(provider, profile, index)))
      await turns(pair, `${provider}-parallel`)
      await input(pair[0], `/account ${profiles[1]}`)
      pair[0].profileId = profiles[1]
      const deadline = Date.now() + 30000
      let switched = false
      while (Date.now() < deadline) {
        const session = (await request("GetSessionState", { session_id: pair[0].sessionId })).session
        if (session.agents.find(agent => agent.id === pair[0].agentId)?.account_profile === profiles[1]) { switched = true; break }
        await sleep(250)
      }
      assert(switched, "prompt-area /account switch did not apply")
      await state(pair[1])
      await capture(pair[0], "account-switched")
      await turns([pair[0]], `${provider}-switched`)
    }
  }
  const after = await scopedAccounts("after")
  assert.deepEqual(after.map(value => value.identity_sha256), before.map(value => value.identity_sha256), "account identities changed")
  result.passed = true
} catch (error) {
  await Promise.allSettled(cells.filter(cell => cell.page).map(cell => capture(cell, "failure")))
  result.passed = false
  result.failure = error instanceof Error ? error.message : "account check failed"
  process.exitCode = 1
} finally {
  result.cleanup = []
  for (const cell of cells) {
    try {
      if (cell.process?.exitCode === null) {
        assert(Number.isSafeInteger(cell.process.pid) && cell.process.pid > 1, "invalid owned TUI PID")
        cell.process.kill("SIGTERM")
        await Promise.race([cell.process.exited, sleep(2000)])
        if (cell.process.exitCode === null) {
          assert(Number.isSafeInteger(cell.process.pid) && cell.process.pid > 1, "invalid owned TUI PID")
          cell.process.kill("SIGKILL")
          await cell.process.exited
        }
      }
      // Recover only our random alias if attachment failed before recording ID.
      const session = (await request("ListSessions")).sessions.find(value => value.alias === cell.alias)
      if (session) await request("DeleteSession", { session_ref: session.id, workspace_id: session.workspace_id })
      assert(!(await request("ListSessions")).sessions.some(value => value.alias === cell.alias), "owned session remains")
      result.cleanup.push({ key: cell.key, tui_stopped: true, session_deleted: true })
    } catch (error) { result.cleanup.push({ key: cell.key, error: error.message }); result.passed = false; process.exitCode = 1 }
  }
  try { await browser?.close() } catch (error) { result.cleanup.push({ browser_error: error.message }); result.passed = false; process.exitCode = 1 }
  frontend?.stop(true)
  await client.close()
  if (scratch) { await rm(scratch, { recursive: true, force: true }); result.cleanup.push({ disk_scratch_removed: true }) }
  result.finished = new Date().toISOString()
  await writeFile(path.join(evidence, "result.json"), JSON.stringify(result, null, 2))
  console.log(JSON.stringify({ items: result.items, source, passed: result.passed, mode: result.mode, evidence }))
}
