/** MP-08/MP-10/MP-11: human-only login UI. Never include these lines in traces. */
export type RuntimeProviderLogin = {
  kernel_id: string
  login: {
    provider: string
    account_profile: string
    login_kind: string
    login_id?: string | null
    auth_url?: string | null
    verification_url?: string | null
    user_code?: string | null
  }
  terminal_output_base64: string
}

export function providerLoginLines(projection: RuntimeProviderLogin): string[] {
  const lines = [`Login runs on ${projection.kernel_id}.`]
  const url = projection.login.verification_url ?? projection.login.auth_url
  if (url && safeProviderLoginUrl(url)) lines.push(url)
  if (projection.login.user_code) lines.push(`Code: ${projection.login.user_code}`)
  if (projection.terminal_output_base64) {
    try {
      const bytes = Uint8Array.from(atob(projection.terminal_output_base64), c => c.charCodeAt(0))
      lines.push(new TextDecoder().decode(bytes).replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "").replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "").replace(/[\x00-\x08\x0b-\x1f\x7f]/g, ""))
    } catch { lines.push("Provider login output is unavailable.") }
  }
  return lines
}

export function safeProviderLoginUrl(value: string): string | null {
  try {
    const url = new URL(value)
    return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password ? url.href : null
  } catch { return null }
}
