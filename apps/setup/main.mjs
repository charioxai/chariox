// MP-07 / MP-08 / MP-11: generic Setup executable; build injects public release inputs only.
import { readFile } from "node:fs/promises"
import { createInterface } from "node:readline/promises"
import { stdin, stdout } from "node:process"
import { installLocal, targetPlatform, publicUrl } from "./installer.mjs"
import { command } from "../kernel/ssh-machine/remote.mjs"

export async function runSetup(build, argv = process.argv.slice(2)) {
  const options = { ...build }
  let login = false, enroll = false
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    if (arg === "--version") { stdout.write(`Chariox Setup ${build.version}\n`); return }
    if (arg === "--help") { stdout.write("MP-07/MP-08/MP-11: Chariox Setup [--repair|--upgrade|--uninstall] [--enroll] [--install-only --login] [--id ID --port PORT]\n--enroll reads a single-use code from stdin or a hidden terminal prompt; codes are never accepted in argv.\n"); return }
    if (arg === "--enroll") { enroll = true; continue }
    if (arg === "--login") { login = true; continue }
    if (arg === "--install-only") { options.installOnly = true; continue }
    const actions = { "--repair": "repair", "--upgrade": "upgrade", "--uninstall": "remove" }
    if (actions[arg]) { if (options.action) throw new Error("choose one Setup action"); options.action = actions[arg]; continue }
    const names = { "--id": "installId", "--port": "port", "--release-version": "version", "--api-url": "apiUrl", "--release-base": "releaseBase", "--user-id": "userId" }
    if (!names[arg] || !argv[i + 1] || argv[i + 1].startsWith("--")) throw new Error("invalid Setup arguments; enrollment codes belong on stdin")
    const value = argv[++i]
    options[names[arg]] = arg === "--port" ? Number(value) : value
  }
  // argv must contain only public selections; never accept or echo an enrollment argument.
  if (enroll) {
    options.installOnly = false; login = false
    if (!stdin.isTTY) {
      let bytes = Buffer.alloc(0)
      try { for await (const chunk of stdin) { if (bytes.length + chunk.length > 4097) throw new Error("enrollment code exceeded limit"); bytes = Buffer.concat([bytes, chunk]) }; options.ticket = bytes.toString("utf8").trim() }
      finally { bytes.fill(0) }
    }
    if (!options.ticket) {
      const { Writable } = await import("node:stream")
      const { openSync } = await import("node:fs")
      const { ReadStream, WriteStream } = await import("node:tty")
      let input = stdin, terminalOutput = stdout, ownTerminal = false
      if (!stdin.isTTY) {
        // curl | sh consumes script stdin. Prompt once on the controlling terminal.
        try { input = new ReadStream(openSync("/dev/tty", "r")); terminalOutput = new WriteStream(openSync("/dev/tty", "w")); ownTerminal = true }
        catch { throw new Error("enrollment code required on stdin or a controlling terminal") }
      }
      let muted = false
      const output = new Writable({ write(chunk, _encoding, done) { if (!muted) terminalOutput.write(chunk); done() } })
      const rl = createInterface({ input, output, terminal: true })
      terminalOutput.write("Single-use enrollment code: "); muted = true
      try { options.ticket = (await rl.question("")).trim() }
      finally { rl.close(); terminalOutput.write("\n"); if (ownTerminal) { input.destroy(); terminalOutput.end() } }
    }
    if (!options.ticket || options.ticket.length > 4096 || /\s/.test(options.ticket)) throw new Error("one bounded enrollment code required")
  }
  options.openBrowser = async url => {
    publicUrl(url.split("?")[0])
    // URL/code are public device-flow verification, never a kernel credential.
    try { await command(process.platform === "darwin" ? "open" : "xdg-open", [url]) } catch { /* printed URL remains available */ }
  }
  const result = await installLocal(options)
  stdout.write(`MP-07/MP-08/MP-11: Chariox ${result.status}; install=${result.installId}\n`)
  if (login) {
    const { spawn } = await import("node:child_process")
    const path = `${options.home ?? process.env.HOME}/.local/bin/${!options.installId || options.installId === "local" ? "chariox" : `chariox-${options.installId}`}`
    await new Promise((yes, no) => {
      const child = spawn(path, ["login", "--api-url", options.apiUrl ?? "https://chariox.com"], { stdio: "inherit" })
      child.once("error", () => no(new Error("installed CLI login could not start")))
      child.once("close", code => code === 0 ? yes() : no(new Error("installed CLI login failed")))
    })
  }
}
