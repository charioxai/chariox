import { kernelUnixClient } from "./kernel-unix-client.js"
import type { KernelSudoRequestedResponse } from "@chariox/kernel-client/kernel-types"

export function parseSudoRequest(argv: string[]) {
  const values = new Map<string, string>()
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i]!
    const value = argv[i + 1]
    if (!["--agent", "--prompt", "--socket"].includes(key) || !value || values.has(key)) {
      throw new Error("Usage: chariox sudo request --agent <id> --prompt <prompt> [--socket <absolute-path>]")
    }
    values.set(key, value)
  }
  const agent_id = values.get("--agent")
  const prompt = values.get("--prompt")
  if (!agent_id?.trim() || !prompt?.trim()) throw new Error("Sudo requires a target agent id and a prompt")
  return { request: { RequestKernelSudo: { agent_id, prompt } }, socket: values.get("--socket") }
}

export async function runSudoCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "sudo") return false
  if (argv[1] !== "request") throw new Error("Usage: chariox sudo request --agent <id> --prompt <prompt> [--socket <absolute-path>]")
  const { request, socket } = parseSudoRequest(argv.slice(2))
  const client = kernelUnixClient(socket)
  try {
    console.error("Waiting for the host's sudo popup. Enter the passkey only in a Chariox terminal.")
    const result = await client.send<KernelSudoRequestedResponse>(request)
    console.log(JSON.stringify(result.KernelSudoRequested))
  } finally { await client.close() }
  return true
}
