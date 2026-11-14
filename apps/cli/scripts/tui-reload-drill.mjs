#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 dataplane/TUI-RELOAD (Cloud docs/CLIENT_DATA_PLANE.md 11.5): restarts the built TUI against a real kernel and
// records its screen every 50 ms in a private tmux server. Faults: F0 none, F1 the kernel answers only 8 s after the TUI started, F7 the kernel
// is down, F10 the kernel restarted between the two visits.
// Each restart passes when the remembered Room view is the first content drawn (F0, F1, F10 within 300 ms of the first frame; F7 at all),
// no frame says the kernel is offline before there is evidence, and nothing of the transcript appears and then disappears.
// Real path: the kernel binary, its local auth, the built TUI (apps/cli/dist) under bun. The provider is the kernel's dev-stub.
//
//   node apps/cli/scripts/tui-reload-drill.mjs --kernel-bin <dir with chariox-kernel> --state <dir outside every repository> --evidence <dir> [--faults F0,F1,F7,F10] [--restarts 5]
import assert from "node:assert/strict"
import { execFileSync, spawn } from "node:child_process"
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs"
import net from "node:net"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, "../../..")
const arg = (name, fallback) => { const index = process.argv.indexOf(`--${name}`); return index >= 0 ? process.argv[index + 1] : fallback }
const kernelBin = path.resolve(arg("kernel-bin", "") || assert.fail("--kernel-bin is required"), "chariox-kernel")
const stateRoot = path.resolve(arg("state", "") || assert.fail("--state is required (a directory outside every repository)"))
const evidence = path.resolve(arg("evidence", "") || assert.fail("--evidence is required"))
assert.ok(path.relative(repo, stateRoot).startsWith(".."), "--state must be outside the repository")
const faults = arg("faults", "F0,F1,F7,F10").split(",")
const restarts = Number(arg("restarts", "5"))
const observeMs = 10_000
// Words that say the kernel is gone. The TUI may say so only with evidence; a TUI that is still opening its link has none.
const offline = /offline|not reachable|cannot be reached|connection ended|disconnected|kernel is down/i
const marker = "drill tui message"

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const runDir = path.join(stateRoot, `tui-reload-${process.pid}`)
const home = path.join(runDir, "chariox-home")
mkdirSync(home, { recursive: true }); mkdirSync(evidence, { recursive: true })
const tmuxServer = `tui-reload-${process.pid}`
const tmux = (...args) => execFileSync("tmux", ["-L", tmuxServer, ...args], { encoding: "utf8" })
const log = []
const note = (line) => { const stamped = `${new Date().toISOString()} ${line}`; log.push(stamped); console.log(stamped) }
const freePort = () => new Promise((resolve, reject) => { const server = net.createServer(); server.on("error", reject); server.listen(0, "127.0.0.1", () => { const { port } = server.address(); server.close(() => resolve(port)) }) })
const ports = { kernel: await freePort(), mcp: await freePort(), opencode: await freePort(), codex: await freePort() }
const env = {
  PATH: process.env.PATH, HOME: path.join(runDir, "home"), TERM: "xterm-256color", LANG: "C.UTF-8", TMPDIR: runDir, CHARIOX_HOME: home,
  XDG_CONFIG_HOME: path.join(runDir, "xdg-config"), XDG_DATA_HOME: path.join(runDir, "xdg-data"), XDG_STATE_HOME: path.join(runDir, "xdg-state"),
  CHARIOX_KERNEL_PORT: String(ports.kernel), CHARIOX_MCP_PORT: String(ports.mcp), CHARIOX_OPENCODE_PORT: String(ports.opencode), CHARIOX_CODEX_PORT: String(ports.codex),
  CHARIOX_DAEMON_SOCKET: path.join(runDir, "daemon.sock"), CHARIOX_PROVIDER_DEV_STUB: "1", CHARIOX_DAEMON_ALIAS: `tui-reload-${process.pid}`,
}
mkdirSync(env.HOME, { recursive: true })

let kernel = null
const startKernel = () => {
  kernel = spawn(kernelBin, [], { cwd: runDir, env, stdio: "ignore" })
  note(`kernel pid ${kernel.pid}`)
}
const stopKernel = async () => {
  if (!kernel || kernel.exitCode !== null || kernel.signalCode !== null) return
  const pid = kernel.pid
  assert.ok(Number.isInteger(pid) && pid > 1)
  process.kill(pid, "SIGTERM")
  for (let waited = 0; kernel.exitCode === null && kernel.signalCode === null; waited += 100) { if (waited > 30_000) process.kill(pid, "SIGKILL"); await sleep(100) }
  note(`kernel ${pid} stopped`)
}
process.env.CHARIOX_HOME = home
const { LocalIpcClient } = await import(pathToFileURL(path.join(repo, "packages/kernel-client/dist/ipc.js")).href)
const requests = await import(pathToFileURL(path.join(repo, "packages/kernel-client/dist/ipc-requests.js")).href)
const kernelUrl = `ws://127.0.0.1:${ports.kernel}`
const answering = async () => {
  for (let attempt = 0; attempt < 300; attempt++) {
    const client = new LocalIpcClient(`${kernelUrl}/kernel`)
    try { await client.send({ ListSessions: null }); return client } catch { await client.close?.().catch(() => {}); await sleep(200) }
  }
  throw new Error("the kernel did not answer within 60 s")
}

const cells = []
try {
  startKernel()
  const client = await answering()
  const workspace = path.join(runDir, "workspace"); mkdirSync(workspace, { recursive: true })
  const created = await client.send(requests.createSessionRequest(workspace, workspace, "tui_reload_room", { provider: "dev-stub", model: "native-tui-idle" }))
  const room = created.SessionCreated.session
  const attached = await client.send({ AttachSessionView: { session_id: room.id, client_id: "tui-reload-seed", capability_level: "FullTerminal", preferred_agent_id: null } })
  for (let turn = 1; turn <= 12; turn++) await client.send({ SubmitPrompt: { session_id: room.id, attachment_id: attached.SessionViewAttached.attachment.id, target_agent_id: room.agents[0].id, prompt: `${marker} ${turn}`, attachments: [] } })
  await client.close?.()
  note(`room ${room.id} with 12 prompts`)
  const session = "tui"
  const launch = (url) => tmux("new-session", "-d", "-s", session, "-x", "140", "-y", "40", `env ${Object.entries(env).map(([key, value]) => `${key}=${JSON.stringify(value)}`).join(" ")} bun ${path.join(repo, "apps/cli/dist/index.js")} --kernel-url ${url} --session ${room.id}; echo "TUI exited with status $?"; sleep 60`)
  const screen = () => { try { return tmux("capture-pane", "-p", "-t", session) } catch { return "" } }
  const close = async () => { try { tmux("send-keys", "-t", session, "C-c") } catch {} await sleep(800); try { tmux("kill-session", "-t", session) } catch {} }
  // The first visit fills the TUI's view cache.
  launch(kernelUrl)
  for (let waited = 0; !screen().includes(`${marker} 12`); waited += 200) { if (waited > 60_000) throw new Error(`the TUI never showed the Room:\n${screen()}`); await sleep(200) }
  await sleep(3_000)
  await close()
  for (const fault of faults) {
    const runs = []
    for (let index = 0; index < restarts; index++) {
      if (fault === "F7") await stopKernel()
      if (fault === "F10") { await stopKernel(); startKernel(); await (await answering()).close?.() }
      // F1: the kernel answers only 8 s after the TUI started (it is coming up); the TUI dials the same address throughout.
      if (fault === "F1") { await stopKernel(); setTimeout(startKernel, 8_000) }
      launch(kernelUrl)
      const began = Date.now()
      const frames = []
      while (Date.now() - began < observeMs) {
        const text = screen()
        if (frames.at(-1)?.text !== text) frames.push({ t: Date.now() - began, text })
        await sleep(50)
      }
      await close()
      if (fault === "F7") startKernel()
      if (fault === "F7" || fault === "F1") await (await answering()).close?.()
      const blank = (text) => text.trim() === ""
      const first = frames.find((frame) => !blank(frame.text))
      const content = frames.find((frame) => frame.text.includes(marker))
      const before = frames.filter((frame) => content ? frame.t < content.t : true).filter((frame) => !blank(frame.text))
      let shown = 0, vanished = 0
      for (const frame of frames) { const lines = (frame.text.match(new RegExp(`${marker} \\d+`, "g")) ?? []).length; if (shown > 0 && lines === 0) vanished++; shown = Math.max(shown, lines) }
      const run = { firstFrameMs: first?.t ?? null, contentMs: content ? content.t : null, contentAfterFirstMs: content && first ? content.t - first.t : null, framesBeforeContent: before.length, offlineFrames: frames.filter((frame) => offline.test(frame.text)).length, vanished, exited: frames.find((frame) => frame.text.includes("TUI exited"))?.text.match(/TUI exited with status \d+/)?.[0] ?? null, frames: frames.length }
      runs.push(run)
      // The first restart of each fault, and every restart that missed, keeps its screens as evidence.
      if (index === 0 || run.contentMs === null || run.exited || run.framesBeforeContent > 0 || run.vanished > 0) {
        const dir = path.join(evidence, `frames-${fault}${index === 0 ? "" : `-restart${index + 1}`}`)
        mkdirSync(dir, { recursive: true })
        frames.forEach((frame, number) => writeFileSync(path.join(dir, `${String(number).padStart(3, "0")}-${frame.t}ms.txt`), frame.text))
      }
      note(`${fault} restart ${index + 1}: ${JSON.stringify(run)}`)
      await sleep(1_000)
    }
    const failures = []
    if (runs.some((run) => run.contentMs === null)) failures.push(`${runs.filter((run) => run.contentMs === null).length} restarts never drew the remembered Room`)
    if (fault !== "F7" && runs.some((run) => run.contentAfterFirstMs !== null && run.contentAfterFirstMs > 300)) failures.push(`remembered Room later than 300 ms after the first frame (max ${Math.max(...runs.map((run) => run.contentAfterFirstMs ?? 0))} ms)`)
    if (runs.some((run) => run.framesBeforeContent > 0)) failures.push(`${runs.reduce((sum, run) => sum + run.framesBeforeContent, 0)} frames drew something else before the remembered Room`)
    if (runs.some((run) => run.offlineFrames > 0) && fault !== "F7") failures.push(`${runs.reduce((sum, run) => sum + run.offlineFrames, 0)} frames say the kernel is offline without evidence`)
    if (runs.some((run) => run.exited)) failures.push(`${runs.filter((run) => run.exited).length} restarts ended the TUI within ${observeMs} ms`)
    if (runs.some((run) => run.vanished > 0)) failures.push(`${runs.reduce((sum, run) => sum + run.vanished, 0)} frames lost the transcript after showing it`)
    cells.push({ fault, runs, failures })
    note(`${fault}: ${failures.length ? `FAIL ${failures.join("; ")}` : "PASS"}`)
  }
} finally {
  try { tmux("kill-server") } catch {}
  await stopKernel()
  writeFileSync(path.join(evidence, "tui-reload.json"), JSON.stringify({ kernelBin, faults, restarts, cells }, null, 1))
  writeFileSync(path.join(evidence, "tui-reload.log"), `${log.join("\n")}\n`)
  writeFileSync(path.join(evidence, "tui-reload.md"), ["| Fault | Restarts | Remembered Room after first frame ms (max) | Frames before it | Offline frames | Lost transcript frames | Verdict |", "|---|---|---|---|---|---|---|",
    ...cells.map(({ fault, runs, failures }) => `| ${fault} | ${runs.length} | ${Math.max(...runs.map((run) => run.contentAfterFirstMs ?? Infinity))} | ${runs.reduce((sum, run) => sum + run.framesBeforeContent, 0)} | ${runs.reduce((sum, run) => sum + run.offlineFrames, 0)} | ${runs.reduce((sum, run) => sum + run.vanished, 0)} | ${failures.length ? `FAIL: ${failures.join("; ")}` : "PASS"} |`)].join("\n") + "\n")
  rmSync(runDir, { recursive: true, force: true })
  if (cells.length < faults.length || cells.some((cell) => cell.failures.length)) process.exitCode = 1
}
