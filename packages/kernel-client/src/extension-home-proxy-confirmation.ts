import type { AgentInstance, ExtensionKind, ExtensionSource } from "./kernel-types.js"

export type HomeProxyGrantConfirmationInput = {
  action: "grant" | "revoke"
  kind: ExtensionKind
  name: string
  source: ExtensionSource
  agent: AgentInstance
  command: string
  confirmed: boolean
}

export function homeProxyGrantConfirmation(
  input: HomeProxyGrantConfirmationInput,
): string | null {
  if (
    input.action !== "grant"
    || input.kind === "skill"
    || input.source !== "home"
    || !input.agent.remote_execution
    || input.confirmed
  ) {
    return null
  }
  const rerun = input.command.includes("--confirm-home-proxy")
    ? input.command
    : `${input.command} --confirm-home-proxy`
  return [
    `Confirm exposing ${input.kind} ${input.name} to remote agent ${input.agent.agent_ref}; home keeps credentials local and executes calls on this machine.`,
    `rerun: ${rerun}`,
  ].join("\n")
}
