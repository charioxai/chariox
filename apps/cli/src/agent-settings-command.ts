import type { AgentInstance } from "@chariox/kernel-client/kernel-types"
import { CommandFailure, parseArguments, usageOf, wantsHelp } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"
import { reportFailure, withAgentKernel } from "./kernels-command.js"

type SettingsAgent = Pick<AgentInstance,
  "id" | "session_id" | "agent_ref" | "alias" | "provider" | "model" | "effort" | "account_profile" |
  "execution_mode_override" | "permission_level_override" | "worktree_id" | "pending_profile">
type SettingsApplies = "now" | "at_step_boundary" | "at_turn_end"

function describeSettings(agent: SettingsAgent, applies?: SettingsApplies) {
  return {
    agent: { id: agent.id, agent_ref: agent.agent_ref, alias: agent.alias },
    settings: {
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort ?? null,
      account_profile: agent.account_profile ?? "default",
      mode: agent.execution_mode_override ?? null,
      permission: agent.permission_level_override ?? null,
      directory: agent.worktree_id,
    },
    pending: agent.pending_profile ?? null,
    ...(applies == null ? {} : {
      status: applies === "at_turn_end"
        ? (agent.pending_profile ? "pending until this turn ends" : "applied when this turn ends")
        : applies === "at_step_boundary" ? "applied, continuing" : "applied",
    }),
  }
}

async function runAgentSettings(
  set: boolean,
  { flags, positionals }: ReturnType<typeof parseArguments>,
  send: SendRequest,
) {
  const flag = (name: string) => flags.get(name) as string | undefined
  const provider = flag("--provider")
  const accountProfile = flag("--account-profile")
  const model = flag("--model")
  const effort = flag("--effort")
  const clearEffort = flags.has("--clear-effort")
  if (set && (!provider && !accountProfile && !model && !effort && !clearEffort)) {
    throw new CommandFailure("usage", "choose a provider, account profile, model or effort to change")
  }
  if (effort && clearEffort) throw new CommandFailure("usage", "choose --effort or --clear-effort")

  const { AgentSettings: { agent } } = await send({
    GetAgentSettings: { session_id: flag("--session"), agent: positionals[0] },
  }) as { AgentSettings: { agent: SettingsAgent } }
  if (!set) return describeSettings(agent)
  const request = {
    session_id: agent.session_id,
    agent_id: agent.id,
    provider,
    account_profile: accountProfile,
    model,
    effort,
    clear_effort: clearEffort,
    reason: flag("--reason"),
  }
  const operation = flags.has("--at-turn-end") ? "UpdateAgentProfileAtTurnEnd" : "UpdateAgentProfile"
  const response = await send({ [operation]: request }) as {
    AgentProfileUpdated: { agent: AgentInstance; applies?: SettingsApplies }
  }
  return describeSettings(response.AgentProfileUpdated.agent, response.AgentProfileUpdated.applies ?? "now")
}

export async function runAgentSettingsCommand(argv: readonly string[]): Promise<boolean> {
  if (argv[0] !== "agent" || argv[1] !== "settings") return false
  const set = argv[2] === "set"
  const command = set ? "agent settings set" : "agent settings"
  const args = argv.slice(2)
  if (wantsHelp(args)) {
    console.log(usageOf(command))
    return true
  }
  let kernel: string | undefined
  try {
    const parsed = parseArguments(command, set ? args.slice(1) : args)
    const { flags } = parsed
    kernel = flags.get("--kernel") as string | undefined
    const socket = flags.get("--socket") as string | undefined
    const commandId = flags.get("--command-id") as string | undefined
    const result = await withAgentKernel({ kernel, socket, commandId }, send => runAgentSettings(set, parsed, send))
    console.log(JSON.stringify(result))
  } catch (error) {
    reportFailure(error, kernel ?? null)
  }
  return true
}
