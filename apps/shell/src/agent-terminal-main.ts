import { runAgentTerminalJsonl, runAgentTerminalServer } from "./agent-terminal.js"

const endpoint = process.env.CHARIOX_KERNEL_URL
  ?? `ws://${process.env.CHARIOX_KERNEL_HOST ?? "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT ?? "43118"}/kernel`

try {
  const run = process.argv.includes("--jsonl") ? runAgentTerminalJsonl : runAgentTerminalServer
  await run({
    endpoint,
    relayAuthToken: process.env.CHARIOX_RELAY_AUTH_TOKEN,
    targetDaemonId: process.env.CHARIOX_RELAY_TARGET_DAEMON_ID,
    targetDaemonAlias: process.env.CHARIOX_RELAY_TARGET_DAEMON_ALIAS,
    homeKernelEndpoint: process.env.CHARIOX_AGENT_TERMINAL_HOME_KERNEL_URL,
    targetKernelRef: process.env.CHARIOX_AGENT_TERMINAL_KERNEL_REF,
    targetMachineRef: process.env.CHARIOX_AGENT_TERMINAL_MACHINE_REF,
    targetSessionId: process.env.CHARIOX_AGENT_TERMINAL_SESSION_ID,
  })
} catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
}
