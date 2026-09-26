#!/usr/bin/env node
// Focused drill for App views (protocol 346): OpenAppView opens an installed
// App in the Room browser, and the page's `window.chariox.call` reaches the
// App's tools through the kernel's call pump. It also checks that App calls
// and other Room commands can overlap: the call poll and answers must not
// make Room commands fail with "already has an active" operation errors.
// Protocol 351: the page reserves a private conversation panel and the Room
// snapshot marks the App Tab with it, in desktop pixels, for the focus agent.
// Protocol 357: the App Tab's accessibility outline lists every node after its
// parent. Opening the App again shows the same, single App Tab.
//
// Runs against a live kernel with a Room bound to a local Docker slice and an
// installed, running App:
//   node scripts/live-app-view-drill.mjs --session ID --installation ID \
//     --slice-id ID --container NAME --tool LOCAL_TOOL [--input JSON] \
//     [--calls 20] [--kernel-url ws://127.0.0.1:44240/kernel] [--evidence FILE]

import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { writeFile } from "node:fs/promises"
import { promisify } from "node:util"

import { LocalIpcClient } from "../dist/ipc.js"

const run = promisify(execFile)
// A lost answer leaves the page's promise pending, so each call gets a
// deadline and reports NO_ANSWER instead of hanging the drill.
const CALL_DEADLINE_MS = 30_000
const CALL = (tool, input) => `Promise.race([
  window.chariox.call(${JSON.stringify(tool)}, ${JSON.stringify(input)}),
  new Promise((_, reject) => setTimeout(() => reject(Object.assign(new Error("no answer"), { code: "NO_ANSWER" })), ${CALL_DEADLINE_MS})),
])`
const options = parseArgs(process.argv.slice(2))
const client = new LocalIpcClient(options.kernelUrl)
const evidence = { started_at: new Date().toISOString(), steps: [] }

try {
  const opened = await client.send({
    OpenAppView: { session_id: options.session, installation_id: options.installation },
  })
  const view = opened.AppViewOpened
  assert.ok(view, `OpenAppView answered ${JSON.stringify(opened)}`)
  assert.equal(view.installation_id, options.installation)
  assert.match(view.origin, /^https:\/\/a[0-9a-f]{24,}\.app\.chariox\.internal\/?$/)
  assert.equal(typeof view.target_id, "string")
  evidence.steps.push({ step: "open", view })

  const ok = await pageCall(view.target_id, options.tool, options.input)
  assert.equal(ok.ok, true, `${options.tool} failed: ${JSON.stringify(ok)}`)
  evidence.steps.push({ step: "call", tool: options.tool, result: ok.value })

  const unknown = await pageCall(view.target_id, "no_such_tool", {})
  assert.deepEqual([unknown.ok, unknown.code], [false, "UNKNOWN_TOOL"])
  evidence.steps.push({ step: "unknown_tool", error: unknown })

  const reserved = await evaluate(view.target_id, `window.chariox.panel.reserve({ x: 880, y: 0, width: 400, height: 800 })`)
  assert.deepEqual(reserved, { reserved: true })
  const marked = await roomApps((apps) => apps.find((app) => app.panel))
  assert.deepEqual({ ...marked.panel, agent_id: undefined }, { x: 880, y: 0, width: 400, height: 800, agent_id: undefined })
  assert.equal(marked.installation_id, options.installation)
  assert.equal(marked.panel.agent_id, view.bound_agent_id ?? null)
  assert.deepEqual(await evaluate(view.target_id, `window.chariox.panel.release()`), { released: true })

  const tabId = await roomAppTab()
  const read = await client.send({ GetRoomEnvironmentTabAccessibility: { session_id: options.session, tab_id: tabId } })
  const outline = read.RoomEnvironmentTabAccessibility?.accessibility
  assert.ok(outline, `GetRoomEnvironmentTabAccessibility answered ${JSON.stringify(read)}`)
  assert.deepEqual([outline.session_id, outline.tab_id], [options.session, tabId])
  assert.ok(outline.nodes.length > 0, "the App view has no readable content")
  const listed = new Set()
  for (const node of outline.nodes) {
    assert.ok(!node.parent_ref || listed.has(node.parent_ref), `${node.element_ref} comes before its parent`)
    listed.add(node.element_ref)
  }
  evidence.steps.push({ step: "outline", tab_id: tabId, nodes: outline.nodes.length, truncated: outline.truncated })

  const again = (await client.send({
    OpenAppView: { session_id: options.session, installation_id: options.installation },
  })).AppViewOpened
  assert.equal(again?.target_id, view.target_id, "opening the App again showed another Tab")
  assert.equal(await roomApps((apps) => apps.length), 1, "the Room holds more than one Tab of this App")
  evidence.steps.push({ step: "reopen", target_id: again.target_id })
  await roomApps((apps) => apps.every((app) => !app.panel))
  evidence.steps.push({ step: "panel", marked })

  // Room commands run while the page makes calls back to back.
  let calling = true
  const room = { ok: 0, busy: 0, other: [] }
  const roomLoop = (async () => {
    while (calling) {
      try {
        const answer = await client.send({
          GetRoomEnvironmentResourceInventory: {
            session_id: options.session,
            slice_id: options.sliceId,
          },
        })
        if (answer.RoomEnvironmentResourceInventory) room.ok += 1
        else room.other.push(answer)
      } catch (error) {
        if (String(error?.message ?? error).includes("already has an active")) room.busy += 1
        else room.other.push(String(error?.message ?? error))
      }
    }
  })()
  let burst
  try {
    burst = await pageBurst(view.target_id, options.tool, options.input, options.calls)
  } finally {
    calling = false
    await roomLoop
  }
  evidence.steps.push({ step: "overlap", burst, room })
  assert.equal(burst.failed.length, 0, `App calls failed: ${JSON.stringify(burst.failed)}`)
  assert.ok(room.ok > 0, "no Room command ran during the App calls")
  assert.equal(room.busy, 0, `${room.busy} Room commands were refused as busy`)
  assert.deepEqual(room.other, [])

  evidence.result = "passed"
  console.log(`App view drill passed: ${burst.ok} calls, ${room.ok} overlapping Room commands`)
} catch (error) {
  evidence.result = "failed"
  evidence.error = String(error?.stack ?? error)
  process.exitCode = 1
  console.error(error)
} finally {
  evidence.finished_at = new Date().toISOString()
  if (options.evidence) await writeFile(options.evidence, `${JSON.stringify(evidence, null, 2)}\n`)
  client.close?.()
}

// Waits until the Room snapshot's Tabs of this installation satisfy `ready`
// and returns its result.
async function roomApps(ready) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const state = await client.send({ GetRoomEnvironmentState: { session_id: options.session } })
    const apps = (state.RoomEnvironmentState?.environment?.tabs ?? [])
      .map((tab) => tab.app).filter((app) => app?.installation_id === options.installation)
    const result = apps.length > 0 && ready(apps)
    if (result) return result
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error("the Room never showed the expected App Tabs")
}

// The Room Tab id of this installation's App view.
async function roomAppTab() {
  const state = await client.send({ GetRoomEnvironmentState: { session_id: options.session } })
  const tab = (state.RoomEnvironmentState?.environment?.tabs ?? [])
    .find((candidate) => candidate.app?.installation_id === options.installation)
  assert.ok(tab, "the Room has no Tab of this App")
  return tab.tab_id
}

async function pageCall(targetId, tool, input) {
  return evaluate(targetId, `(async () => {
    try { return { ok: true, value: await ${CALL(tool, input)} } }
    catch (error) { return { ok: false, code: error.code, message: error.message } }
  })()`)
}

async function pageBurst(targetId, tool, input, count) {
  return evaluate(targetId, `(async () => {
    const failed = []
    let ok = 0
    for (let index = 0; index < ${count}; index += 1) {
      try { await ${CALL(tool, input)}; ok += 1 }
      catch (error) { failed.push({ index, code: error.code, message: error.message }) }
    }
    return { ok, failed }
  })()`)
}

// Evaluates in the App's tab through the slice browser's DevTools endpoint.
async function evaluate(targetId, expression) {
  const script = `
    const [targetId, expression] = process.argv.slice(1)
    const targets = await (await fetch("http://127.0.0.1:9222/json/list")).json()
    const target = targets.find((entry) => entry.id === targetId)
    if (!target) throw new Error("App tab not found: " + targetId)
    const socket = new WebSocket(target.webSocketDebuggerUrl)
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject })
    socket.send(JSON.stringify({ id: 1, method: "Runtime.evaluate",
      params: { expression, awaitPromise: true, returnByValue: true } }))
    const keepAlive = setInterval(() => {}, 1000)
    const reply = await new Promise((resolve, reject) => {
      socket.onmessage = (event) => { const message = JSON.parse(event.data); if (message.id === 1) resolve(message) }
      socket.onclose = () => reject(new Error("DevTools socket closed before the answer"))
    }).finally(() => clearInterval(keepAlive))
    socket.close()
    if (reply.error || reply.result.exceptionDetails) throw new Error(JSON.stringify(reply.error ?? reply.result.exceptionDetails))
    console.log(JSON.stringify(reply.result.result.value))
  `
  const { stdout, stderr } = await run(
    "docker",
    ["exec", options.container, "node", "--input-type=module", "-e", script, targetId, expression],
    { timeout: 60_000 + CALL_DEADLINE_MS * options.calls },
  )
  const line = stdout.trim().split("\n").at(-1)
  if (!line) throw new Error(`the App tab gave no answer: ${stderr.trim()}`)
  return JSON.parse(line)
}

function parseArgs(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 2) {
    assert.match(argv[index] ?? "", /^--/, `unexpected argument ${argv[index]}`)
    values[argv[index].slice(2)] = argv[index + 1]
  }
  for (const name of ["session", "installation", "slice-id", "container", "tool"]) {
    assert.ok(values[name], `--${name} is required`)
  }
  return {
    kernelUrl: values["kernel-url"] ?? process.env.KERNEL_URL ?? "ws://127.0.0.1:44240/kernel",
    session: values.session,
    installation: values.installation,
    sliceId: values["slice-id"],
    container: values.container,
    tool: values.tool,
    input: JSON.parse(values.input ?? "{}"),
    calls: Number(values.calls ?? 20),
    evidence: values.evidence,
  }
}
