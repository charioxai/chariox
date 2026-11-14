import type { QueuedJob } from "@chariox/kernel-client/queued-jobs"

/** The current Room is supplied at the click, so a retained stripe cannot target a Room the person left. */
export async function performQueuedJobAction(client: { send<T>(request: unknown): Promise<T> }, sessionId: string, job: QueuedJob, action: "remote" | "stop"): Promise<void> {
  if (action === "remote" && !job.remoteAvailable) throw Error("This command cannot run remotely.")
  const request = action === "remote" ? { RunSessionProcessRemotely: { session_id: sessionId, process_id: job.processId } } : { StopSessionProcesses: { session_id: sessionId, target: { kind: "process", process_id: job.processId } } }
  const reply = await client.send<Record<string, unknown>>(request)
  if (!(action === "remote" ? reply.SessionProcessRemoteStarted : reply.SessionProcessesStopped)) throw Error("The kernel did not confirm the command action.")
}
