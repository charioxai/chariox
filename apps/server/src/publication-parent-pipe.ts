import type { Readable } from "node:stream"

/// Set by the kernel that launches a gateway: the gateway's stdin is a pipe
/// the kernel holds open for as long as it runs. The OS closes it when the
/// kernel dies for any reason (including SIGKILL), so the gateway never
/// outlives its kernel.
export const PUBLICATION_PARENT_PIPE_ENV = "CHARIOX_PUBLICATION_EXIT_ON_STDIN_CLOSE"

export function exitWhenParentPipeCloses(
  env: NodeJS.ProcessEnv,
  stdin: Readable,
  exit: () => void,
): boolean {
  if (env[PUBLICATION_PARENT_PIPE_ENV] !== "1") return false
  stdin.once("end", exit)
  stdin.once("error", exit)
  // Nothing is sent on the pipe; drain it so `end` is delivered.
  stdin.resume()
  return true
}
