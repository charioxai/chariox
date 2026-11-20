import { runCommandRequest } from "@chariox/kernel-client/ipc-requests"
import type { SendRequest } from "./kernel-files.js"
import { CommandFailure } from "./agent-command-table.js"

export type ExecPlaceOptions = { session: string; execId: string; kernel: string; directory?: string | undefined }

/** Runs a launch that is still held on this kernel on another of the owner's kernels. */
export async function runExecPlace(options: ExecPlaceOptions, send: SendRequest): Promise<{ exec_id: string }> {
  const response = await send(runCommandRequest(
    { kind: "room_workspace", session_id: options.session },
    { operation: "exec_place", exec_id: options.execId, kernel: options.kernel, ...(options.directory ? { directory: options.directory } : {}) },
  )) as { RunCommand: { result: { result: string; exec_id?: string } } }
  const result = response.RunCommand.result
  if (result.result !== "exec_opened" || !result.exec_id) throw new CommandFailure("exec_place_unreadable", "the kernel did not confirm the placement; read the launch with exec_list before placing it again")
  return { exec_id: result.exec_id }
}
