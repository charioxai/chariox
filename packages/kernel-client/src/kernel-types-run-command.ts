export type CommandTarget =
  | { readonly kind: "kernel" }
  | { readonly kind: "room_workspace"; readonly session_id: string }
  | { readonly kind: "agent_slice"; readonly session_id: string; readonly agent_id: string }

export type CommandExit = { kind: "not_started" } | { kind: "exited"; code: number } | { kind: "signaled"; signal: number }
export type ExecOutcome = { kind: "exited"; code: number } | { kind: "signaled"; signal: number } | { kind: "memory_limit"; limit_bytes: number } | { kind: "failed"; message: string } | { kind: "timed_out"; timeout_ms: number; exit: CommandExit }
export type ExecOutput = { chunks: readonly { cursor: number; stream: "stdout" | "stderr"; data_base64: string }[]; next_cursor: number; outcome: ExecOutcome | null }

export type ExecState = { kind: "queued"; wait: { reason: "memory" | "disk"; need_bytes: number; free_bytes: number } } | { kind: "starting" | "running" | "finished" }
export type ExecSummary = { exec_id: string; state: ExecState; age_ms: number; buffered_bytes: number; command_line: string; command: readonly string[]; owner: { kind: "agent" | "terminal"; id: string }; /** Kernels a held launch could run on instead, for the caller that may place it. */ placeable_on?: readonly string[] }

export type RunCommandOperation =
  | { readonly operation: "exec_list" }
  | { readonly operation: "exec_open"; readonly command: readonly string[]; readonly directory?: string; readonly timeout_ms?: number; readonly stdin_file?: string }
  | { readonly operation: "exec_open_job"; readonly command: readonly string[]; readonly needs?: { readonly memory_gib?: number; readonly cpus?: number } }
  | { readonly operation: "exec_write"; readonly exec_id: string; readonly offset: number; readonly data_base64: string }
  | { readonly operation: "exec_stdin_close"; readonly exec_id: string; readonly offset: number }
  | { readonly operation: "exec_read"; readonly exec_id: string; readonly after_cursor: number; readonly wait_ms?: number }
  | { readonly operation: "exec_cancel" | "exec_close" | "exec_start_now"; readonly exec_id: string }
  | { readonly operation: "exec_place"; readonly exec_id: string; readonly kernel: string; readonly directory?: string }

export type RunCommandRequest = {
  readonly target: CommandTarget
  readonly acting_agent?: string
  readonly operation: RunCommandOperation
}

export type RunCommandResult =
  | { readonly result: "exec_listed"; readonly execs: readonly ExecSummary[] }
  | { readonly result: "exec_opened"; readonly exec_id: string }
  | { readonly result: "exec_input"; readonly next_offset: number }
  | { readonly result: "exec_output"; readonly output: ExecOutput }
  | { readonly result: "exec_cancelled" }
  | { readonly result: "exec_closed" }
