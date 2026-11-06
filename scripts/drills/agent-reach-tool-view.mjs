// MP-08 / MP-09 / MP-10 / MP-11 agent reach: the tools a provider shows its agent, seen two ways.
// 1. The two MCP servers an agent run is launched with are read from the real provider process of the drill kernel
//    and asked for their tool lists over HTTP: the `chariox` list must not change, `chariox-reach` has the tools.
// 2. The provider's own interactive tool view (Claude `/mcp`, Codex `/mcp`, OpenCode `/status`) is started in tmux with
//    those servers and photographed (headless Chrome, device pixel ratio 1) for the evidence.
// Bearers stay inside this process and the lane's own scratch directory; they are never printed.
import { execFileSync, spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"

const childrenOf = pid => {
  const out = []
  for (const task of readdirSync(`/proc/${pid}/task`)) {
    try { out.push(...readFileSync(`/proc/${pid}/task/${task}/children`, "utf8").split(" ").filter(Boolean).map(Number)) } catch {}
  }
  return out
}
const argvOf = pid => readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0").filter(Boolean)
const envOf = pid => Object.fromEntries(readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").filter(entry => entry.includes("=")).map(entry => [entry.slice(0, entry.indexOf("=")), entry.slice(entry.indexOf("=") + 1)]))

/** The `chariox` and `chariox-reach` servers the kernel launched this provider with: `{ name: { url, headers } }`. */
export function launchedServers(provider, kernelPid) {
  // Codex runs as `node codex app-server`; the one with MCP overrides is the run's own.
  const ownProcess = pid => argvOf(pid).slice(0, 2).some(arg => arg === provider || arg.endsWith(`/${provider}`)) && (provider !== "codex" || argvOf(pid).some(arg => arg.startsWith("mcp_servers.chariox-reach.")))
  const child = childrenOf(kernelPid).find(ownProcess)
  if (!child) throw new Error(`no ${provider} process under the kernel`)
  const argv = argvOf(child)
  if (provider === "claude") {
    const file = argv[argv.indexOf("--mcp-config") + 1]
    return { servers: JSON.parse(readFileSync(file, "utf8")).mcpServers, file, argv, pid: child }
  }
  if (provider === "opencode") {
    return { servers: JSON.parse(envOf(child).OPENCODE_CONFIG_CONTENT).mcp, argv, pid: child }
  }
  // Codex: `-c mcp_servers.<name>.url="..."` overrides and the bearer in the environment variable they name.
  const overrides = argv.flatMap((arg, i) => argv[i - 1] === "-c" ? [arg] : [])
  const env = envOf(child)
  const servers = {}
  for (const name of ["chariox", "chariox-reach"]) {
    const value = key => overrides.find(entry => entry.startsWith(`mcp_servers.${name}.${key}=`))?.slice(`mcp_servers.${name}.${key}=`.length)
    const url = JSON.parse(value("url") ?? "null"), tokenVar = JSON.parse(value("bearer_token_env_var") ?? "null")
    if (url && tokenVar) servers[name] = { url, headers: { Authorization: `Bearer ${env[tokenVar]}` } }
  }
  return { servers, overrides: overrides.filter(entry => entry.startsWith("mcp_servers.")), env, argv, pid: child }
}

export async function toolNames(server) {
  const response = await fetch(server.url, { method: "POST", headers: { ...server.headers, "Content-Type": "application/json", Accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }) })
  const text = await response.text()
  const payload = JSON.parse(text.startsWith("event:") || text.startsWith("data:") ? text.split("\n").find(line => line.startsWith("data:")).slice(5) : text)
  return (payload.result?.tools ?? []).map(tool => tool.name)
}

export const sha = names => createHash("sha256").update(JSON.stringify([...names].sort())).digest("hex")

const COLORS = ["#000", "#c33", "#3a3", "#cb3", "#38c", "#c3c", "#3cc", "#ccc", "#666", "#f55", "#5f5", "#ff5", "#58f", "#f5f", "#5ff", "#fff"]
function color256(n) {
  if (n < 16) return COLORS[n]
  if (n >= 232) { const v = 8 + (n - 232) * 10; return `rgb(${v},${v},${v})` }
  const c = n - 16, level = x => (x ? 55 + x * 40 : 0)
  return `rgb(${level(Math.floor(c / 36))},${level(Math.floor(c / 6) % 6)},${level(c % 6)})`
}
/** A tmux capture with its SGR colours as an HTML page. */
export function ansiPage(ansi, cols, rows, title) {
  const esc = text => text.replace(/&/g, "&amp;").replace(/</g, "&lt;")
  let style = {}, html = ""
  const css = () => {
    const fg = style.inverse ? style.bg ?? "#000" : style.fg ?? "#ddd", bg = style.inverse ? style.fg ?? "#ddd" : style.bg
    return `color:${fg};${bg ? `background:${bg};` : ""}${style.bold ? "font-weight:bold;" : ""}${style.dim ? "opacity:.65;" : ""}${style.underline ? "text-decoration:underline;" : ""}`
  }
  for (const part of ansi.split(/(\x1b\[[0-9;:]*m)/)) {
    const match = /^\x1b\[([0-9;:]*)m$/.exec(part)
    if (!match) { if (part) html += `<span style="${css()}">${esc(part)}</span>`; continue }
    const codes = match[1].split(/[;:]/).map(code => (code === "" ? 0 : Number(code)))
    for (let i = 0; i < codes.length; i++) {
      const code = codes[i]
      if (code === 0) style = {}
      else if (code === 1) style.bold = true
      else if (code === 2) style.dim = true
      else if (code === 4) style.underline = true
      else if (code === 7) style.inverse = true
      else if (code >= 30 && code <= 37) style.fg = COLORS[code - 30]
      else if (code >= 90 && code <= 97) style.fg = COLORS[code - 82]
      else if (code >= 40 && code <= 47) style.bg = COLORS[code - 40]
      else if (code >= 100 && code <= 107) style.bg = COLORS[code - 92]
      else if (code === 39) delete style.fg
      else if (code === 49) delete style.bg
      else if ((code === 38 || code === 48) && codes[i + 1] === 5) { style[code === 38 ? "fg" : "bg"] = color256(codes[i + 2]); i += 2 }
      else if ((code === 38 || code === 48) && codes[i + 1] === 2) { style[code === 38 ? "fg" : "bg"] = `rgb(${codes[i + 2]},${codes[i + 3]},${codes[i + 4]})`; i += 4 }
    }
  }
  const width = Math.ceil(cols * 8.4) + 24, height = rows * 17 + 40
  return { width, height, html: `<!doctype html><meta charset="utf-8"><title>${esc(title)}</title><body style="margin:0;background:#111"><div style="padding:8px 12px;font:12px sans-serif;color:#9a9;background:#1b1b1b">${esc(title)}</div><pre style="margin:0;padding:8px 12px;font:14px/17px 'DejaVu Sans Mono',monospace;color:#ddd;background:#111;white-space:pre">${html}</pre></body>` }
}

export function screenshot(page, path) {
  const file = path.replace(/\.png$/, ".html")
  writeFileSync(file, page.html)
  const run = spawnSync("google-chrome", ["--headless=new", "--no-sandbox", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1", `--window-size=${page.width},${page.height}`, `--screenshot=${path}`, `file://${file}`], { encoding: "utf8", timeout: 60000 })
  if (run.status !== 0) throw new Error(`screenshot failed: ${run.stderr.slice(0, 200)}`)
}

const tmux = (...args) => execFileSync("tmux", ["-L", "reach-view", ...args], { encoding: "utf8" })

/** Start the provider's own TUI in tmux, drive `steps` and photograph each `shot`. Returns the captures. */
export async function photographTui({ name, program, env, cwd, steps, ready, dialogs = [], cols = 140, rows = 42, evidence, label }) {
  const session = `reach-${name}-${process.pid}`
  const envArgs = Object.entries(env).flatMap(([key, value]) => ["-e", `${key}=${value}`])
  tmux("new-session", "-d", "-s", session, "-x", String(cols), "-y", String(rows), "-c", cwd, ...envArgs, ...program)
  const shots = []
  try {
    // First-run dialogs of the provider's TUI, answered until its prompt shows.
    const end = Date.now() + 60000
    while (ready && Date.now() < end) {
      const screen = tmux("capture-pane", "-p", "-t", session)
      if (ready.test(screen)) break
      const dialog = dialogs.find(entry => entry.when.test(screen))
      if (dialog) for (const key of dialog.keys) tmux("send-keys", "-t", session, ...(key.literal ? ["-l", key.literal] : [key]))
      await delay(2000)
    }
    for (const step of steps) {
      if (step.keys) for (const key of step.keys) tmux("send-keys", "-t", session, ...(key.literal ? ["-l", key.literal] : [key]))
      await delay(step.wait ?? 2000)
      if (step.until) {
        const end = Date.now() + (step.timeout ?? 30000)
        while (Date.now() < end && !step.until.test(tmux("capture-pane", "-p", "-t", session))) await delay(500)
      }
      if (step.shot) {
        const ansi = tmux("capture-pane", "-e", "-p", "-t", session)
        const plain = tmux("capture-pane", "-p", "-t", session)
        writeFileSync(join(evidence, `${step.shot}.txt`), plain)
        screenshot(ansiPage(ansi, cols, rows, `${label} · ${step.shot} · DPR 1`), join(evidence, `${step.shot}.png`))
        shots.push({ shot: step.shot, plain })
      }
    }
  } finally {
    try { tmux("kill-session", "-t", session) } catch {}
  }
  return shots
}

export function ensureDirectory(path) { mkdirSync(path, { recursive: true, mode: 0o700 }); return path }
