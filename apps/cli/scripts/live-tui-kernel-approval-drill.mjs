#!/usr/bin/env bun
// Drives the real TUI in a pseudo-terminal: a kernel approval raised while the
// TUI is attached must show the live indicator, F8 must open the panel, and
// arrow keys plus Enter must reach the kernel. Fails on Solid's server build,
// where the global key handler and the indicator effect never run.
import { createHash, generateKeyPairSync, randomUUID } from "node:crypto"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const usage = "usage: bun apps/cli/scripts/live-tui-kernel-approval-drill.mjs --kernel-url URL --session ID [--cli dist/index.js]"
const options = { cli: path.join(cliRoot, "dist/index.js") }
for (let index = 2; index < process.argv.length; index += 2) {
  const [flag, value] = process.argv.slice(index, index + 2)
  if (flag === "--kernel-url") options.kernelUrl = value
  else if (flag === "--session") options.session = value
  else if (flag === "--cli") options.cli = path.resolve(value)
  else throw new Error(usage)
}
if (!options.kernelUrl || !options.session) throw new Error(usage)

const { LocalIpcClient } = await import(path.join(cliRoot, "dist/ipc.js"))
const { AppPublisherEnrollment } = await import(path.join(cliRoot, "dist/app-publisher-file.js"))
const scratch = await mkdtemp(path.join(os.tmpdir(), "chariox-tui-approval-"))
const client = new LocalIpcClient(options.kernelUrl, {})
const enrollment = new AppPublisherEnrollment((request) => client.send(request), () => {}, scratch)
const keys = { f8: "\x1b[19~", up: "\x1b[A", enter: "\r", ctrlE: "\x05" }

let output = ""
// Approximate screen text: OpenTUI writes each new text run contiguously.
const strip = (text) => text.replace(/\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b[()][0-9A-Za-z]|\x1b[=>78]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)/g, "")
const tui = Bun.spawn([process.execPath, options.cli, "--kernel-url", options.kernelUrl, "--session", options.session], {
  env: { ...process.env, TERM: "xterm-256color" },
  terminal: { cols: 160, rows: 48, data(_terminal, chunk) { output += new TextDecoder().decode(chunk) } },
})
const screen = () => strip(output)
const since = () => { const mark = output.length; return () => strip(output.slice(mark)) }
const waitFor = async (text, read = screen, timeoutMs = 30_000) => {
  const deadline = Date.now() + timeoutMs
  while (!read().includes(text)) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${JSON.stringify(text)}`)
    await Bun.sleep(100)
  }
}
const press = async (key) => { tui.terminal.write(key); await Bun.sleep(400) }
const step = (name) => console.log(`[tui-approval-drill] ${name}`)

let requestId = null
try {
  await waitFor("Ctrl+T hotkeys")
  step("attached")
  const { publicKey } = generateKeyPairSync("ed25519")
  const key = publicKey.export({ format: "der", type: "spki" }).subarray(-32)
  await writeFile(path.join(scratch, "publisher.json"), JSON.stringify({
    schema: "chariox.developer-publisher.v1", algorithm: "ed25519", publicKey: key.toString("base64"),
    publisher: { id: `tui-drill-${randomUUID().slice(0, 8)}`, name: "TUI approval drill",
      keyId: `dev-${createHash("sha256").update(key).digest("hex")}` },
  }))
  const indicator = since()
  requestId = (await enrollment.enroll("publisher.json", options.session)).request_id
  await waitFor("1 approval", indicator)
  step("indicator refreshed while attached")
  if (screen().includes("Chariox approval 1 of 1")) throw new Error("the approval panel opened before F8")
  const panel = since()
  await press(keys.f8)
  await waitFor("Chariox approval 1 of 1", panel, 5_000)
  step("F8 opened the approval panel")
  const selected = since()
  await press(keys.up)
  await waitFor("Selected: Cancel", selected, 5_000)
  await press(keys.enter)
  const deadline = Date.now() + 10_000
  let phase = "pending"
  while (phase === "pending" && Date.now() < deadline) {
    await Bun.sleep(250)
    phase = (await enrollment.status(requestId)).phase
  }
  if (phase !== "denied") throw new Error(`expected the kernel to record the declined review, got ${phase}`)
  step("arrow and Enter declined the review in the kernel")
  await press(keys.ctrlE)
  const code = await Promise.race([tui.exited, Bun.sleep(5_000).then(() => null)])
  if (code === null) throw new Error("Ctrl+E did not exit the TUI")
  step("passed")
} finally {
  if (requestId) await enrollment.cancel(requestId).catch(() => {})
  tui.kill()
  await client.close().catch(() => {})
  await rm(scratch, { recursive: true, force: true })
}
