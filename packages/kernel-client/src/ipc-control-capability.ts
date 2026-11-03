function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : undefined
}

// Query the connected kernel, not relay discovery metadata. Parallel development
// branches can reuse a numeric protocol version without implementing this contract.
export async function requireKernelControlCapability(
  send: (request: unknown) => Promise<unknown>,
  request: unknown,
  expected?: { daemonId: string; machineId?: string },
): Promise<void> {
  const envelope = record(request)
  if (!envelope) return
  const names = Object.keys(envelope)
  const name = names.find(value => value === "CancelActivePrompt" && record(envelope[value])?.expected_prompt_id != null)
  if (!name) return
  if (names.length !== 1) throw new Error("Invalid kernel control request")
  const response = record(await send({ RelayStatus: null }))
  const status = record(record(response?.RelayStatus)?.status)
  if (!status || typeof status.daemon_id !== "string" || !status.daemon_id
    || typeof status.machine_id !== "string" || !status.machine_id) {
    throw new Error("Kernel control capability status is unavailable")
  }
  if (expected && status.daemon_id !== expected.daemonId) {
    throw new Error("Kernel control capability status belongs to another kernel")
  }
  if (expected?.machineId !== undefined && status.machine_id !== expected.machineId) {
    throw new Error("Kernel control capability status belongs to another machine")
  }
  const capability = "exact_prompt_cancellation_v499"
  if (!Array.isArray(status.capabilities)
    || !status.capabilities.every(value => typeof value === "string")
    || !status.capabilities.includes(capability)) {
    throw new Error(`Connected kernel does not support ${capability}; update the kernel before retrying`)
  }
}
