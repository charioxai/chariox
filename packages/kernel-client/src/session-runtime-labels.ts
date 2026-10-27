export type SessionHomeKernelLabelInput = {
  readonly host_daemon_id?: string | null | undefined
  readonly host_machine_id?: string | null | undefined
  readonly kernel_id?: string | null | undefined
  readonly homeKernelId?: string | null | undefined
  readonly homeMachineId?: string | null | undefined
}

export function formatSessionHomeKernelLabel(
  session: SessionHomeKernelLabelInput | null | undefined,
  fallback = "-",
): string {
  const kernel = session?.host_daemon_id?.trim()
    || session?.kernel_id?.trim()
    || session?.homeKernelId?.trim()
    || ""
  const machine = session?.host_machine_id?.trim()
    || session?.homeMachineId?.trim()
    || ""
  if (kernel && machine) {
    return `${kernel}@${machine}`
  }
  return kernel || machine || fallback
}
