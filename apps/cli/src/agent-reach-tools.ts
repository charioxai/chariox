// MP-08 / MP-09 / MP-10 / MP-11 reach: the tools of the `chariox-reach` MCP server, taken from the same command
// table as the CLI and its guide, so the three never drift. `scripts/generate-reach-tools.mjs` writes the result
// where the kernel embeds it; a test fails while that file is out of date.
import { COMMANDS, REQUEST_EXAMPLES } from "./agent-command-table.js"

type Param = { name: string; type: "string" | "integer" | "object"; description: string; required?: true; maximum?: number }
type Tool = {
  name: string
  /** The CLI command the tool does the work of; every flag of it is a param or is named in `without`. */
  command?: string
  description: string
  params: readonly Param[]
  /** Flags of the command the tool does not offer, with the reason. */
  without?: Readonly<Record<string, string>>
}

const SOCKET = { "--socket": "the tool runs on your own kernel" }
const KERNEL: Param = { name: "kernel", type: "string", description: "Kernel name or id from `kernels`; omit for this kernel." }

export const REACH_TOOLS: readonly Tool[] = [
  { name: "kernels", command: "kernels", params: [], without: SOCKET, description: "List the kernels of your user that you can reach from this room: name, id, machine and state." },
  {
    name: "sessions",
    params: [KERNEL, { name: "session_id", type: "string", description: "A room of that kernel: list its agents instead of the rooms." }],
    description: "List the rooms (sessions) of a kernel, or with `session_id` the agents of one room. The ids you need for `request` and `events_subscribe`.",
  },
  {
    name: "request",
    command: "request",
    params: [KERNEL, { name: "request", type: "object", required: true, description: "One externally tagged kernel request, for example {\"ListSessions\":null}. `guide` lists examples." }],
    without: { ...SOCKET, "--command-id": "the kernel numbers your calls", "--no-subscribe": "not needed in a tool", "--timeout": "the call lasts as long as the request" },
    description: "Send one kernel request for your user to another room or kernel. Without /sudo you may do there what a Room agent without /sudo may do in its own room: list and read sessions, agents and history, attach and message agents, use a room's terminals and files, and use machines. What needs /sudo in your own room needs a live /sudo window of your own turn here; owner-only requests are refused. Example: {\"kernel\":\"my-mac\",\"request\":{\"ListSessions\":null}}.",
  },
  {
    name: "events_subscribe",
    command: "events subscribe",
    params: [
      { ...KERNEL, required: true, description: "Kernel name or id from `kernels`." },
      { name: "session_id", type: "string", required: true, description: "The room to follow." },
      { name: "agent_id", type: "string", description: "Follow only this agent of the room." },
    ],
    without: { ...SOCKET, "--source": "notification sources are followed by your user's own outside agent, never by a room agent" },
    description: "Follow a room, or one agent of it, in your event inbox; `events_wait` brings what happens there.",
  },
  {
    name: "events_wait",
    command: "events wait",
    params: [
      { name: "since", type: "integer", description: "Records after this cursor; omit for all pending records." },
      { name: "limit", type: "integer", maximum: 1000, description: "At most this many records (default 1000)." },
      { name: "timeout_seconds", type: "integer", maximum: 280, description: "How long to wait for a record (default 60, at most 280: a provider ends a tool call that lasts longer)." },
    ],
    without: SOCKET,
    description: "Wait for pending records of the rooms and agents you follow; returns at once when some are pending and `timed_out` when none came. Keep next_cursor.",
  },
  {
    name: "events_ack",
    command: "events ack",
    params: [{ name: "cursor", type: "integer", required: true, description: "Acknowledge all records through this cursor." }],
    without: SOCKET,
    description: "Acknowledge the records you handled, through a cursor `events_wait` returned.",
  },
  { name: "guide", command: "guide", params: [], description: "How reach works: the tools, example requests, authority and refusal codes." },
]

/** Requests an agent may send; the rest of the CLI's examples are the user's alone or end a room. */
const REACH_EXAMPLES = ["ListSessions", "ListProjects", "SearchWorkspaceDirectories", "CreateSession", "AttachToSession", "GetSessionState", "SpawnAgent", "SubmitPrompt", "CancelActivePrompt", "RespondToInteraction"]

const requestName = (body: object): string => Object.keys(body)[0]!

export function reachExamples(): { request: object; note: string }[] {
  return REQUEST_EXAMPLES.filter(example => REACH_EXAMPLES.includes(requestName(example.body))).map(example => ({ request: example.body, note: example.note }))
}

function schemaOf(tool: Tool): object {
  const required = tool.params.filter(param => param.required).map(param => param.name)
  return {
    type: "object",
    properties: Object.fromEntries(tool.params.map(param => [param.name, { type: param.type, description: param.description, ...(param.maximum === undefined ? {} : { maximum: param.maximum }) }])),
    ...(required.length ? { required } : {}),
    additionalProperties: false,
  }
}

export function reachGuideText(): string {
  const tools = REACH_TOOLS.map(tool => `  ${tool.name}(${tool.params.map(param => `${param.name}${param.required ? "" : "?"}`).join(", ")}): ${tool.description}`)
  return [
    "Chariox reach guide for Room agents. The chariox tools act in your own room; the chariox-reach tools act on other rooms and kernels of your user.",
    "Tools:",
    ...tools,
    "Find a kernel with kernels (its name is the machine name, or <machine>/<label>), its rooms with sessions, then send requests with request. Without a kernel argument a request runs on this kernel.",
    "Requests for request (replace the UPPERCASE values; ids come from earlier answers):",
    ...reachExamples().map(({ request, note }) => `  ${JSON.stringify(request)}  | ${note}`),
    "A request the kernel cannot parse comes back with a message naming the expected variants and fields: read it and fix the request.",
    "Follow rooms or single agents with events_subscribe, then events_wait and events_ack with the cursor you keep.",
    "A record names kernel, session_id, agent_id and refs (interaction_id, run_id, task_id). A record with value.question is a decision the person may answer through you: ask the person (options are question.choices, a free reply is question.text_reply when present),",
    "then send request RespondToInteraction with refs.interaction_id and the chosen id to the kernel the record names. Answer without asking only for an agent your user told you to drive and how to decide. You never answer a decision of your own room.",
    "A record with a pointer and no question is the person's alone: show pointer.one_line verbatim.",
    "Read what happened in any room of your user, without asking: request {\"GetSessionHistoryOutline\":{\"session_id\":\"SESSION_ID\",\"agent_ids\":[\"AGENT_ID\"],\"latest_prompt_count\":4,\"include_prefetched_content\":true}} (SessionHistoryOutline.agents[].turns and prefetched_blobs).",
    "Refusals are results with a code: reach_requires_sudo (needs your user's /sudo window, as it would in your own room), excluded_for_agents and human_only (never yours), not_allowed, reach_not_available (your run cannot reach), acquisition_declined (the user declined a machine), and the kernel_* codes of an unreachable or slow kernel: request retries a lost or timed-out remote reply once internally with the original command ID; on kernel_outcome_unknown read the state before starting another operation.",
    "Machines of your user's connected service: request {\"ControlPlane\":{\"action\":\"capabilities\"}} gives machine_allocation.revision, then {\"ControlPlane\":{\"action\":\"acquire\",\"capability_revision\":\"REVISION\",\"request_id\":\"ANY_NEW_ID\"}} (the user is asked to accept first unless your turn already has a live /sudo window or the setting is off; the call waits for the answer, and acquisition_declined means no).",
    "Then {\"ControlPlane\":{\"action\":\"get\",\"allocation_ref\":\"REF\"}} every ten seconds or so until state is ready and access_state is available; kernel_target.kernel_id is the machine's kernel for the kernel argument. stop, start and release take allocation_ref, capability_revision, a new request_id and expected_revision (the revision of the latest answer). Release the machine when you are done.",
    "A new machine has no room. Create one on it with CreateSession (workspace_id and worktree_id are a directory on the machine, such as /home/chariox; no /sudo needed), then the other room requests there.",
    "Run a command in a room with RoomTerminal: request {\"RoomTerminal\":{\"session_id\":\"SESSION_ID\",\"operation\":{\"operation\":\"exec_open\",\"command\":[\"uname\",\"-a\"]}}} answers exec_opened.exec_id; then {\"RoomTerminal\":{\"session_id\":\"SESSION_ID\",\"operation\":{\"operation\":\"exec_read\",\"exec_id\":\"EXEC_ID\",\"after_cursor\":0,\"wait_ms\":10000}}} until exec_output has an outcome (chunks hold base64 stdout and stderr; pass next_cursor as the next after_cursor), and exec_close releases it. An attachment you make is automation-only, so RunShellCommand and the other requests that need a terminal attachment are refused for you.",
    "Human-only, never attempted by you: passkey approvals, grants, sudo approvals, secrets and credentials, relay and pairing configuration.",
  ].join("\n")
}

/** What the kernel embeds: the tool list and the guide. */
export function reachToolsDocument(): { tools: { name: string; description: string; input_schema: object }[]; examples: { request: object; note: string }[]; guide: string } {
  return {
    tools: REACH_TOOLS.map(tool => ({ name: tool.name, description: tool.description, input_schema: schemaOf(tool) })),
    examples: reachExamples(),
    guide: reachGuideText(),
  }
}

/** Problems that keep the tools from matching the command table; empty when they match. */
export function reachToolProblems(): string[] {
  const problems: string[] = []
  for (const tool of REACH_TOOLS) {
    if (!tool.command) continue
    const spec = COMMANDS.find(entry => entry.name === tool.command)
    if (!spec) { problems.push(`${tool.name}: no command ${tool.command}`); continue }
    for (const flag of spec.flags) {
      if (!tool.without?.[flag.name] && !tool.params.some(param => `--${param.name.replace(/_id$|_seconds$/, "").replaceAll("_", "-")}` === flag.name)) problems.push(`${tool.name}: ${flag.name} of "${tool.command}" is neither a param nor listed in without`)
    }
    for (const arg of spec.args ?? []) {
      if (!tool.params.some(param => param.name === arg.toLowerCase())) problems.push(`${tool.name}: argument ${arg} of "${tool.command}" is not a param`)
    }
    for (const name of Object.keys(tool.without ?? {})) {
      if (!spec.flags.some(flag => flag.name === name)) problems.push(`${tool.name}: without names ${name}, which "${tool.command}" does not take`)
    }
  }
  return problems
}
