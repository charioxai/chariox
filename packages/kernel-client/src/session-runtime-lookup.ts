import type {
  RuntimeInteraction,
  RuntimeProviderRun,
  RuntimeSession,
} from "./kernel-types.js"
import { sessionFocusedAgentId } from "./session-runtime-transition.js"

export function sessionActiveInteractionForAgent(
  session: Pick<RuntimeSession, "active_interactions">,
  agentId: string | null | undefined,
): RuntimeInteraction | null {
  if (!agentId) {
    return null
  }
  return session.active_interactions?.find((interaction) => interaction.agent_id === agentId) ?? null
}

export function sessionFocusedInteraction(
  session: Pick<RuntimeSession, "active_interactions" | "agents" | "focused_agent_id">,
): RuntimeInteraction | null {
  return sessionActiveInteractionForAgent(session, sessionFocusedAgentId(session))
}

export function runtimeProviderRunForAgent(
  run: RuntimeProviderRun | null,
  agentId: string | null | undefined,
): RuntimeProviderRun | null {
  return run && run.agent_instance_id === agentId ? run : null
}

export function sameProviderRun(left: RuntimeProviderRun, right: RuntimeProviderRun): boolean {
  return left.id === right.id
    && left.session_id === right.session_id
    && left.agent_instance_id === right.agent_instance_id
    && left.adapter_key === right.adapter_key
    && left.provider === right.provider
    && left.account_profile === right.account_profile
    && left.model === right.model
    && left.variant === right.variant
    && left.client_interface === right.client_interface
    && left.usage_tokens_total === right.usage_tokens_total
    && left.state === right.state
}
