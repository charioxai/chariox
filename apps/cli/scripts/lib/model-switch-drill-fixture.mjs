// Child-process fixture for the real drill; no kernel, login, or provider process.
import { registerHooks } from "node:module"

const ipc = `
import { existsSync, writeFileSync, unlinkSync } from "node:fs"
import path from "node:path"
export class LocalIpcClient {
  constructor() {
    this.scenario = process.env.CTXSWITCH_FIXTURE_SCENARIO
    this.root = process.env.CTXSWITCH_FIXTURE_ROOT
    this.calls = []
    this.facts = { release_name: "fixture-release" }
  }
  async send(request) {
    const [kind, value] = Object.entries(request)[0]
    if (kind === "CreateSession") {
      this.agent = { ...value.agent_defaults, id: "home-agent" }
      return { SessionCreated: { session: { id: "session" }, agent: this.agent } }
    }
    if (kind === "SpawnAgent") {
      this.agent = { ...value, id: "placed-agent", remote_execution: {
        worker_kernel_id: "worker", execution_lease_id: "lease", leased_agent_id: "worker-agent",
      } }
      return { AgentSpawned: { agent: this.agent } }
    }
    if (kind === "AttachToSession") return { SessionAttached: { attachment: { id: "attachment" } } }
    if (kind === "UpdateAgentProfile") {
      this.calls.push({ label: "switch" })
      Object.assign(this.agent, value)
      return { AgentProfileUpdated: { agent: this.agent } }
    }
    if (kind === "SubmitPrompt") {
      const prompt = value.prompt
      const label = prompt.match(/^\\[(.*)-\\d+\\]/)[1]
      this.calls.push({ label, provider: this.agent.provider, account_profile: this.agent.account_profile,
        agent_id: value.target_agent_id, kernel_ref: this.agent.kernel_ref, slice_ref: this.agent.slice_ref })
      let text = "OK", lifecycle = "completed", tool = false
      if (label === "fact-1") this.facts.codename = prompt.match(/codename is (\\S+)\\./)[1]
      if (label === "fact-2") this.facts.port = prompt.match(/port is (\\d+)/)[1]
      if (label === "fact-3") this.facts.fruit = prompt.match(/is the (\\S+)\\./)[1]
      if (label === "fact-4") this.facts.colour = prompt.match(/colour is (\\S+)\\./)[1]
      if (label === "release") text = this.facts.release_name
      if (label === "decision-new") {
        this.facts.cache_database = prompt.match(/use (\\S+) for/)[1]
        this.facts.current_task = prompt.match(/task is (\\S+),/)[1]
        this.facts.next_step = prompt.match(/step is (\\S+)\\./)[1]
      }
      if (label === "file") {
        this.facts.created_file = prompt.match(/relative file (\\S+) containing/)[1]
        this.file = path.join(this.root, this.facts.created_file)
        writeFileSync(this.file, this.facts.codename)
        if (this.scenario === "file-submit-failure") throw new Error("transport lost after file creation")
        text = this.facts.codename; tool = true
        if (this.scenario === "file-readback-failure") lifecycle = "failed"
      }
      if (label === "file-cleanup") {
        tool = true
        if (this.scenario === "cleanup-tool-failure-matching-reply") {
          // Completed assistant turn, failed shell operation, echoed prompt marker.
          text = "CTXSWITCH_FILE_REMOVED"
        } else if (this.scenario === "cleanup-failure" || (this.scenario === "target-failure" && this.agent.provider === "claude")) {
          lifecycle = "failed"; text = "provider failed"
        } else {
          if (existsSync(this.file)) unlinkSync(this.file)
          text = "CTXSWITCH_FILE_REMOVED"
        }
      }
      if (label === "tool") {
        const [, a, b] = prompt.match(/print\\((\\d+)\\*(\\d+)\\)/)
        this.facts.python_output = text = String(Number(a) * Number(b)); tool = true
      }
      if (label.startsWith("recall")) {
        text = Object.entries(this.facts).map(([key, val]) => key + "=" + val).join(" ")
        if (this.scenario === "target-failure") { lifecycle = "failed"; text = "login expired" }
      }
      const toolFailed = lifecycle !== "completed" || (label === "file-cleanup" && this.scenario === "cleanup-tool-failure-matching-reply")
      this.turn = { user_prompt: { entry: { text: prompt } }, lifecycle, blobs: [], entries: [
        { entry: { kind: "provider_output", text } },
        ...(tool ? [{ entry: { kind: "provider_tool", text: JSON.stringify({
          tool: "bash", input: { command: label === "file" && this.scenario === "unrelated-file-tool"
            ? "pwd" : prompt.match(/Run exactly: ([^\\n]+)/)?.[1] ?? "python3" },
          status: toolFailed ? "error" : "completed",
          raw: "exit_code: " + (toolFailed ? "1" : "0"), output: text,
        }) } }] : []),
      ] }
      return { PromptSubmitted: {} }
    }
    if (kind === "GetSessionHistoryOutline") return { SessionHistoryOutline: { agents: [
      { agent_id: this.agent.id, turns: [structuredClone(this.turn)] },
    ] } }
    if (kind === "DeleteSession") { this.calls.push({ label: "delete" }); return {} }
    throw new Error("unexpected fixture request " + kind)
  }
  async close() {
    writeFileSync(path.join(this.root, "trace.json"), JSON.stringify({ calls: this.calls, fileExists: !!this.file && existsSync(this.file) }))
  }
}
`
// Keep the fixture independent of generated dist artifacts; it tests drill
// sequencing and failure handling, not the already shared builder serialization.
const requests = `
export const createSessionRequest = (workspace, worktree, alias, agent_defaults) => ({ CreateSession: { agent_defaults } })
export const spawnAgentRequest = (session, provider, alias, model, worktree, effort, mode, permissions, kernel_ref, placement, slice_ref, account_profile) => ({ SpawnAgent: { provider, model, effort, kernel_ref, slice_ref, account_profile } })
export const attachToSessionRequest = () => ({ AttachToSession: {} })
export const submitPromptRequest = (session, attachment, target_agent_id, prompt) => ({ SubmitPrompt: { target_agent_id, prompt } })
export const updateAgentProfileRequest = ({ provider, model, effort, accountProfile }) => ({ UpdateAgentProfile: { provider, model, effort, account_profile: accountProfile } })
export const getSessionHistoryOutlineRequest = () => ({ GetSessionHistoryOutline: {} })
export const getSessionHistoryBlobContentRequest = () => ({ GetSessionHistoryBlobContent: {} })
export const deleteSessionRequest = () => ({ DeleteSession: {} })
`
registerHooks({
  resolve(specifier, context, nextResolve) {
    const source = specifier === "../dist/ipc.js" ? ipc : specifier === "../dist/ipc-requests.js" ? requests : null
    return source ? { url: `data:text/javascript,${encodeURIComponent(source)}`, shortCircuit: true } : nextResolve(specifier, context)
  },
})
