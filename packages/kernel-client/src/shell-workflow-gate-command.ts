import { createHash } from "node:crypto"
import { open } from "node:fs/promises"
import { resolve } from "node:path"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import type { WorkflowGateAction, WorkflowGateEvaluation, WorkflowGateEntry } from "./kernel-types.js"

async function file(path: string, limit: number): Promise<Buffer> {
  const handle = await open(path, "r")
  try {
    const stat = await handle.stat()
    if (!stat.isFile() || stat.size > limit) throw new Error(`gate input must be a file of at most ${limit} bytes`)
    const bytes = Buffer.alloc(limit + 1)
    const { bytesRead } = await handle.read(bytes, 0, bytes.length, 0)
    if (bytesRead > limit) throw new Error("gate input grew beyond its limit")
    return bytes.subarray(0, bytesRead)
  } finally { await handle.close() }
}
export async function executeWorkflowGateCommand(
  args: string[], context: ShellContext,
  deps: { client: { send: (request: Record<string, unknown>) => Promise<Record<string, unknown>> } },
): Promise<ShellCommandResult> {
  const [command, workflow, value, extra] = args
  if (!context.sessionId || !workflow) return { ok: false, message: "usage: workflow gate put|test|entry|cancel <workflow> <file|entry-id> [evaluation-file|reason]" }
  let action: WorkflowGateAction
  if (command === "put" && value && !extra) {
    const bytes = await file(resolve(context.worktree || context.workspace, value), 2 * 1024 * 1024)
    action = { kind: "put_module", wasm_base64: bytes.toString("base64"), sha256: `sha256:${createHash("sha256").update(bytes).digest("hex")}` }
  } else if (command === "test" && value && extra) {
    action = { kind: "test", module_sha256: value, evaluation: JSON.parse((await file(resolve(context.worktree || context.workspace, extra), 512 * 1024)).toString("utf8")) as WorkflowGateEvaluation }
  } else if (command === "entry" && value && !extra) action = { kind: "entry", entry_id: value }
  else if (command === "cancel" && value && extra) action = { kind: "cancel", entry_id: value, reason: args.slice(3).join(" ") }
  else if (command === "entries") action = {kind:"entries",after:value ?? null,limit:32}
  else if (command === "state") action = {kind:"state",binding_id:value ?? null}
  else if (command === "reset-state" && value) action = {kind:"reset_state",reason:args.slice(2).join(" ")}
  else return { ok: false, message: "usage: workflow gate put|test|entry|cancel <workflow> <file|entry-id> [evaluation-file|reason]" }
  const response = await deps.client.send({ WorkflowGate: { session_id: context.sessionId, workflow_ref: workflow, action } })
  if ("WorkflowGateEntries" in response) {
    const payload=response.WorkflowGateEntries as {entries:WorkflowGateEntry[];next_cursor:string|null}
    return {ok:true,message:payload.entries.map(e=>`${e.entry_id} [${e.disposition}]`).join("\n")+(payload.next_cursor ? `\nnext cursor: ${payload.next_cursor}` : ""),data:payload}
  }
  if ("WorkflowGateState" in response) return {ok:true,message:JSON.stringify(response.WorkflowGateState),data:response.WorkflowGateState}
  if ("WorkflowGateModuleStored" in response) return { ok: true, message: `stored gate ${(response.WorkflowGateModuleStored as {sha256:string}).sha256}`, data: response.WorkflowGateModuleStored }
  if ("WorkflowGateTested" in response) return { ok: true, message: JSON.stringify(response.WorkflowGateTested), data: response.WorkflowGateTested }
  if ("WorkflowEntryAccepted" in response) {
    const payload = response.WorkflowEntryAccepted as {entry: WorkflowGateEntry}
    return { ok: true, message: `workflow entry ${payload.entry.entry_id} [${payload.entry.disposition}]\n${JSON.stringify(payload.entry)}`, data: payload }
  }
  throw new Error(`gate request failed: ${JSON.stringify(response)}`)
}
