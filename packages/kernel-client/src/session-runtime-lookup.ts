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
    && sameProviderRunUsage(left.usage, right.usage)
    && left.state === right.state
    && left.endpoint_mode === right.endpoint_mode
    && left.process_label === right.process_label
    && left.structured_endpoint === right.structured_endpoint
    && left.provider_session_id === right.provider_session_id
    && left.working_directory === right.working_directory
    && left.started_at_ms === right.started_at_ms
    && left.last_activity_at_ms === right.last_activity_at_ms
    && sameProviderRunControlCapabilities(left.control_capabilities, right.control_capabilities)
    && sameProviderRunExternalImport(left.external_provider_import, right.external_provider_import)
}

function sameProviderRunUsage(
  left: RuntimeProviderRun["usage"],
  right: RuntimeProviderRun["usage"],
): boolean {
  if (!left || !right) {
    return left === right
  }
  return left.total_tokens === right.total_tokens
    && left.last_tokens === right.last_tokens
    && left.context_tokens === right.context_tokens
    && left.context_window === right.context_window
}

function sameProviderRunControlCapabilities(
  left: RuntimeProviderRun["control_capabilities"],
  right: RuntimeProviderRun["control_capabilities"],
): boolean {
  if (!left || !right) {
    return left === right
  }
  return left.length === right.length
    && left.every((capability, index) =>
      capability.operation === right[index]?.operation
      && capability.mode === right[index]?.mode)
}

function sameProviderRunExternalImport(
  left: RuntimeProviderRun["external_provider_import"],
  right: RuntimeProviderRun["external_provider_import"],
): boolean {
  if (!left || !right) {
    return left === right
  }
  return left.external_provider_session_id === right.external_provider_session_id
    && left.external_provider === right.external_provider
    && left.external_provider_session_provider_id === right.external_provider_session_provider_id
    && left.imported_at_ms === right.imported_at_ms
    && left.last_observed_turn_id === right.last_observed_turn_id
    && left.last_observed_at_ms === right.last_observed_at_ms
    && left.observed_cursor.last_observed_turn_id === right.observed_cursor.last_observed_turn_id
    && left.observed_cursor.last_observed_at_ms === right.observed_cursor.last_observed_at_ms
    && left.observed_cursor.last_observed_merge_key === right.observed_cursor.last_observed_merge_key
    && sameStringArray(
      left.observed_cursor.arroba_owned_observed_prompt_turn_ids,
      right.observed_cursor.arroba_owned_observed_prompt_turn_ids,
    )
}

function sameStringArray(left?: readonly string[], right?: readonly string[]): boolean {
  if (!left || !right) {
    return left === right
  }
  return left.length === right.length
    && left.every((value, index) => value === right[index])
}
