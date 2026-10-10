// MP-07 / MP-08 / MP-11: per-user launchd projection for the shared installer lifecycle.
import { join } from "node:path"
import { command, isolatedKernelEnvironment } from "../kernel/ssh-machine/remote.mjs"
const xml = value => String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&apos;")
export function launchdDefinition(id) {
  const name = `com.chariox.kernel.${id}.plist`, label = name.slice(0, -6)
  return { name, relativeDirectory: "Library/LaunchAgents", render(home, request, root, kernelPath) {
    const args = ["/usr/bin/env", ...isolatedKernelEnvironment.flatMap(key => ["-u", key]), `${root}/current/${kernelPath}`]
    const env = { HOME: home, PATH: process.env.PATH ?? "/usr/bin:/bin", CHARIOX_HOME: `${home}/.chariox/dev/ssh-machines/${id}`, CHARIOX_KERNEL_HOST: "127.0.0.1", CHARIOX_KERNEL_PORT: request.port, CHARIOX_MCP_HOST: "127.0.0.1", CHARIOX_MCP_PORT: request.port + 1 }
    return `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict><key>Label</key><string>${label}</string><key>ProgramArguments</key><array>${args.map(arg => `<string>${xml(arg)}</string>`).join("")}</array><key>EnvironmentVariables</key><dict>${Object.entries(env).map(([key, value]) => `<key>${key}</key><string>${xml(value)}</string>`).join("")}</dict><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict></dict></plist>\n`
  } }
}
export function launchdManager(home, id, invoke = command) {
  const definition = launchdDefinition(id), label = definition.name.slice(0, -6)
  const domain = `gui/${process.getuid()}`, service = `${domain}/${label}`, path = join(home, definition.relativeDirectory, definition.name)
  return async args => {
    if (args[0] === "daemon-reload") return ""
    if (args[1] !== definition.name && args[0] === "show") throw new Error("foreign launchd service")
    if (args[0] === "show") {
      let loaded = false, fragment = ""
      try {
        const info = await invoke("launchctl", ["print", service], true)
        const match = /^\s*path = (.+)$/m.exec(info)
        if (!match || match[1] !== path) throw new Error("foreign launchd service")
        fragment = match[1]; loaded = true
      } catch (error) { if (error.message === "foreign launchd service") throw error }
      // Verify the user's session domain even when this label does not exist.
      await invoke("launchctl", ["print", domain])
      let state = ""
      if (args.includes("--property=ActiveState")) {
        const disabled = await invoke("launchctl", ["print-disabled", domain], true)
        const entries = [...disabled.matchAll(/^\s*"([^"]+)"\s*=>\s*(true|false)\s*$/gm)]
        const enabled = !entries.some(entry => entry[1] === label && entry[2] === "true")
        state = `ActiveState=${loaded ? "active" : "inactive"}\nUnitFileState=${enabled ? "enabled" : "disabled"}\n`
      }
      return `LoadState=${loaded ? "loaded" : "not-found"}\nFragmentPath=${fragment}\nDropInPaths=\n${state}`
    }
    if (args.at(-1) !== definition.name) throw new Error("foreign launchd service")
    if (args[0] === "enable") {
      await invoke("launchctl", ["enable", service])
      if (args.includes("--now")) {
        try { await invoke("launchctl", ["print", service]) }
        catch { await invoke("launchctl", ["bootstrap", domain, path]) }
      }
    } else if (args[0] === "start") {
      await invoke("launchctl", ["bootstrap", domain, path])
    } else if (args[0] === "disable" || args[0] === "stop") {
      // A missing job is idempotent; never stop another label or process group.
      let loaded = true
      try { await invoke("launchctl", ["print", service]) } catch { loaded = false }
      if (loaded && (args[0] === "stop" || args.includes("--now"))) await invoke("launchctl", ["bootout", service])
      if (args[0] === "disable") await invoke("launchctl", ["disable", service])
    } else throw new Error("unsupported launchd operation")
    return ""
  }
}
