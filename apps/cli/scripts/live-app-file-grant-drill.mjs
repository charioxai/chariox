#!/usr/bin/env node
// Focused drill for user-selected file grants (protocol 354). The Documents
// App's `import_documents` tool calls `host.pickFile`; the kernel shows the
// owner a trusted prompt in their most recent session, and the owner answers
// with `GrantAppFile`. The App imports the granted copy as a new document.
//
// Runs against a live kernel with a Room bound to a local Docker slice and a
// running Documents installation. SESSION must be the owner's most recent
// session, where the prompt appears:
//   node scripts/live-app-file-grant-drill.mjs --session ID --installation ID \
//     --container NAME [--kernel-url ws://127.0.0.1:44240/kernel] [--evidence FILE]

import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { randomUUID } from "node:crypto"
import { writeFile } from "node:fs/promises"
import { promisify } from "node:util"

import { LocalIpcClient } from "../dist/ipc.js"

const run = promisify(execFile)
const options = parseArgs(process.argv.slice(2))
const client = new LocalIpcClient(options.kernelUrl)
const evidence = { started_at: new Date().toISOString(), steps: [] }
const title = `grant-drill-${randomUUID().slice(0, 8)}`
const base64 = (text) => Buffer.from(text).toString("base64")
const grant = (operationId, files) => client.send({
  GrantAppFile: { session_id: options.session, operation_id: operationId, files },
})
let targetId
let imported

try {
  const opened = (await client.send({
    OpenAppView: { session_id: options.session, installation_id: options.installation },
  })).AppViewOpened
  assert.ok(opened, "OpenAppView did not open the App")
  targetId = opened.target_id
  const requested = await pageCall("import_documents", {})
  assert.deepEqual(requested, { ok: true, value: { requested: true } })

  const prompt = await waitFor(async () => (await interactions()).find((item) => item.id.startsWith("app_file_pick_")))
  const operationId = prompt.id.slice("app_file_pick_".length)
  assert.deepEqual(prompt.choices.map((choice) => choice.id), ["decline"])
  evidence.steps.push({ step: "prompt", operation_id: operationId, title: prompt.title })

  // Refused without using up the pick: a suffix the App did not accept, and
  // an answer larger than one relayed request can carry.
  const refused = await grant(operationId, [{ name: "drill.exe", contents_base64: base64("x") }])
  assert.equal(refused.AppRequestFailed?.code, "invalid_request", JSON.stringify(refused))
  const half = "x".repeat(300 * 1024)
  const oversized = await grant(operationId, [
    { name: "a.md", contents_base64: base64(half) },
    { name: "b.md", contents_base64: base64(half) },
  ])
  assert.equal(oversized.AppRequestFailed?.code, "invalid_request", JSON.stringify(oversized))
  evidence.steps.push({ step: "refused", suffix: refused, total: oversized })

  const granted = await grant(operationId, [{ name: `${title}.md`, contents_base64: base64(`# ${title}\n\nGranted by the drill.\n`) }])
  assert.deepEqual(granted.AppFileGranted, { operation_id: operationId, files: 1 }, JSON.stringify(granted))
  await waitFor(async () => !(await interactions()).some((item) => item.id === prompt.id))
  const again = await grant(operationId, [{ name: `${title}.md`, contents_base64: base64("again") }])
  assert.equal(again.AppRequestFailed?.code, "conflict", JSON.stringify(again))
  evidence.steps.push({ step: "granted", granted, prompt_closed: true, again })

  imported = await waitFor(async () => {
    const listed = await pageCall("list_documents", {})
    return listed.value?.documents?.find((doc) => doc.title === title)
  })
  const read = await pageCall("read_document", { id: imported.id })
  assert.match(read.value?.content ?? "", /Granted by the drill\./)
  evidence.steps.push({ step: "imported", document: imported })

  evidence.result = "passed"
  console.log(`App file grant drill passed: ${title} imported through operation ${operationId}`)
} catch (error) {
  evidence.result = "failed"
  evidence.error = String(error?.stack ?? error)
  process.exitCode = 1
  console.error(error)
} finally {
  if (imported) await pageCall("delete_document", { id: imported.id }).catch(() => {})
  evidence.finished_at = new Date().toISOString()
  if (options.evidence) await writeFile(options.evidence, `${JSON.stringify(evidence, null, 2)}\n`)
  client.close?.()
}

async function interactions() {
  const state = await client.send({ GetSessionState: { session_id: options.session } })
  return state.SessionState?.session?.active_interactions ?? []
}

async function waitFor(check) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const result = await check()
    if (result) return result
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error("timed out waiting")
}

async function pageCall(tool, input) {
  return evaluate(`(async () => {
    try { return { ok: true, value: await window.chariox.call(${JSON.stringify(tool)}, ${JSON.stringify(input)}) } }
    catch (error) { return { ok: false, code: error.code, message: error.message } }
  })()`)
}

// Evaluates in the App's tab through the slice browser's DevTools endpoint.
async function evaluate(expression) {
  const script = `
    const [targetId, expression] = process.argv.slice(1)
    const targets = await (await fetch("http://127.0.0.1:9222/json/list")).json()
    const target = targets.find((entry) => entry.id === targetId)
    if (!target) throw new Error("App tab not found: " + targetId)
    const socket = new WebSocket(target.webSocketDebuggerUrl)
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject })
    socket.send(JSON.stringify({ id: 1, method: "Runtime.evaluate",
      params: { expression, awaitPromise: true, returnByValue: true } }))
    const reply = await new Promise((resolve) => {
      socket.onmessage = (event) => { const message = JSON.parse(event.data); if (message.id === 1) resolve(message) }
    })
    socket.close()
    if (reply.error || reply.result.exceptionDetails) throw new Error(JSON.stringify(reply.error ?? reply.result.exceptionDetails))
    console.log(JSON.stringify(reply.result.result.value))
  `
  const { stdout } = await run(
    "docker",
    ["exec", options.container, "node", "--input-type=module", "-e", script, targetId, expression],
    { timeout: 60_000 },
  )
  return JSON.parse(stdout.trim().split("\n").at(-1))
}

function parseArgs(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 2) {
    assert.match(argv[index] ?? "", /^--/, `unexpected argument ${argv[index]}`)
    values[argv[index].slice(2)] = argv[index + 1]
  }
  for (const name of ["session", "installation", "container"]) assert.ok(values[name], `--${name} is required`)
  return {
    kernelUrl: values["kernel-url"] ?? "ws://127.0.0.1:44240/kernel",
    session: values.session,
    installation: values.installation,
    container: values.container,
    evidence: values.evidence,
  }
}
