/** Expected owner decisions use the kernel's existing structured error code. */
export function ownerRequestNotice(error: unknown): string | null {
  const code = error && typeof error === "object" && "code" in error ? error.code : null
  switch (code) {
    case "sudo_refused": return "Sudo refused in another terminal"
    case "kernel_access_refused": return "Access refused in another terminal"
    case "owner_request_expired": return "Request expired"
    default: return null
  }
}
