import {
  resolveAttachTimeProviderLaunch,
  type SessionLifecycleLaunchSelection,
} from "@chariox/kernel-client/session-lifecycle-state"

import type {
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"

type RuntimeAgent = RuntimeSession["agents"][number]
type SkipReason = "no_visible_agents" | "missing_focused_agent" | "unowned_visible_agent" | "remote_backed_agent"

export type AttachProviderRunSettlement =
  | {
    action: "launched"
    session: RuntimeSession
    providerRun: RuntimeProviderRun
    launch: SessionLifecycleLaunchSelection
    targetAgentId: string | null
  }
  | {
    action: "loaded"
    session: RuntimeSession
    providerRun: RuntimeProviderRun | null
    providerRunId: string
  }
  | {
    action: "skipped"
    session: RuntimeSession
    providerRun: null
    launch: SessionLifecycleLaunchSelection
    reason: SkipReason
    targetAgent: RuntimeAgent | null
    recoveredRemotePlacement: boolean
  }

type AttachProviderRunDeps = {
  launchProviderRun: (
    sessionId: string,
    provider: string,
    accountProfile: string,
    model: string,
    effort: string,
    targetAgentId?: string | null,
  ) => Promise<RuntimeProviderRun>
  getSessionState: (sessionId: string) => Promise<RuntimeSession>
  tryGetProviderRun: (providerRunId: string) => Promise<RuntimeProviderRun | null>
}

export async function settleAttachProviderRun(
  session: RuntimeSession,
  fallback: SessionLifecycleLaunchSelection,
  accountProfile: string,
  createdSession: boolean,
  deps: AttachProviderRunDeps,
): Promise<AttachProviderRunSettlement> {
  const decision = resolveAttachTimeProviderLaunch(session, fallback, createdSession)
  switch (decision.action) {
    case "load_provider_run":
      return {
        action: "loaded",
        session,
        providerRun: await deps.tryGetProviderRun(decision.providerRunId),
        providerRunId: decision.providerRunId,
      }
    case "skip_launch":
      return {
        action: "skipped",
        session,
        providerRun: null,
        launch: decision.launch,
        reason: decision.reason,
        targetAgent: decision.targetAgent,
        recoveredRemotePlacement: false,
      }
    case "launch_provider_run": {
      try {
        return {
          action: "launched",
          session,
          providerRun: await deps.launchProviderRun(
            session.id,
            decision.launch.provider,
            accountProfile,
            decision.launch.model,
            decision.launch.effort,
            decision.targetAgentId,
          ),
          launch: decision.launch,
          targetAgentId: decision.targetAgentId,
        }
      } catch (launchError) {
        let refreshedSession: RuntimeSession
        try {
          refreshedSession = await deps.getSessionState(session.id)
        } catch {
          throw launchError
        }
        // Placement may commit between the attach snapshot and launch request.
        const transitionedAgent = decision.targetAgentId
          ? refreshedSession.agents.find((agent) => agent.id === decision.targetAgentId) ?? null
          : null
        if (!transitionedAgent?.remote_execution) {
          throw launchError
        }
        return {
          action: "skipped",
          session: refreshedSession,
          providerRun: null,
          launch: decision.launch,
          reason: "remote_backed_agent",
          targetAgent: transitionedAgent,
          recoveredRemotePlacement: true,
        }
      }
    }
    default: {
      const exhaustive: never = decision
      throw new Error(`unhandled attach provider launch decision ${String(exhaustive)}`)
    }
  }
}
