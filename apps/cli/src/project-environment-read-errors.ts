// MP-08 / MP-10 / MP-11: keep transport diagnostics out of the read-only panel.
export function environmentReadFailureMessage(cause: unknown): string {
  const message = cause instanceof Error ? cause.message : ""
  return message.includes("project.get") && message.includes("was not found")
    ? "No Environment for this project yet."
    : "Environment unavailable. Check the kernel connection and reopen this panel."
}
