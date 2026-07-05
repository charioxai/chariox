#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..")
const kernelBin = resolve(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const root = await mkdtemp(join(tmpdir(), "arroba-live-trace-kernel."))
const configHome = join(root, "config")
const stateHome = join(root, "state")
const runtimeHome = join(root, "run")
const userConfigDir = join(configHome, "arroba")
const kernelPort = process.env.ARROBA_TRACE_KERNEL_PORT ?? "44135"
const mcpPort = process.env.ARROBA_TRACE_MCP_PORT ?? "44136"

await mkdir(userConfigDir, { recursive: true })
await writeFile(join(userConfigDir, "config.toml"), [
  "[state]",
  `path = ${JSON.stringify(join(root, "kernel-state.db"))}`,
  "",
  "[history.operational]",
  `path = ${JSON.stringify(join(root, "operational-history.db"))}`,
  "",
  "[artifacts.operational]",
  `root = ${JSON.stringify(join(root, "artifacts"))}`,
  `index_path = ${JSON.stringify(join(root, "artifacts", "index.db"))}`,
  "",
].join("\n"))

const env = {
  ...process.env,
  XDG_CONFIG_HOME: configHome,
  XDG_STATE_HOME: stateHome,
  XDG_RUNTIME_DIR: runtimeHome,
  ARROBA_SESSION_HISTORY_DIR: join(root, "sessions"),
  ARROBA_DAEMON_SOCKET: join(runtimeHome, "trace-drill-kernel.sock"),
  ARROBA_KERNEL_PORT: kernelPort,
  ARROBA_MCP_PORT: mcpPort,
  ARROBA_DAEMON_ID: process.env.ARROBA_DAEMON_ID ?? "trace-drill-kernel",
  ARROBA_MACHINE_ID: process.env.ARROBA_MACHINE_ID ?? "trace-drill-machine",
  ARROBA_MACHINE_ALIAS: process.env.ARROBA_MACHINE_ALIAS ?? "trace-drill",
  ARROBA_DAEMON_ALIAS: process.env.ARROBA_DAEMON_ALIAS ?? "trace-drill",
}

process.stdout.write(`ARROBA_TRACE_KERNEL_ROOT=${root}\n`)
process.stdout.write(`ARROBA_KERNEL_URL=ws://127.0.0.1:${kernelPort}/kernel\n`)

const child = spawn(kernelBin, process.argv.slice(2), {
  cwd: repoRoot,
  env,
  stdio: "inherit",
})

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal)
    return
  }
  process.exit(code ?? 1)
})
