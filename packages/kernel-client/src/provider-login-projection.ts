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

const PROVIDER_NAMES: Record<string, string> = { claude: "Claude", codex: "ChatGPT", opencode: "OpenCode" }

export function providerLoginName(provider: string): string {
  return PROVIDER_NAMES[provider] ?? provider
}

/** The response field of a provider-native login waiting for a pasted code
 * (`provider-response`), as opposed to its vault passphrase phases. */
export const PROVIDER_LOGIN_CODE_CHOICE_ID = "provider-response"

export type ProviderLoginView = {
  title: string
  url: string | null
  /** Steps after "Open the link". */
  steps: string[]
  /** The interaction takes a pasted code from the provider's page. */
  takesCode: boolean
}

/** MP-08/MP-11: one human-readable view of a provider login interaction. */
export function providerLoginView(
  projection: RuntimeProviderLogin,
  customChoiceId: string | null | undefined,
  accountLabel = projection.login.account_profile,
): ProviderLoginView {
  const name = providerLoginName(projection.login.provider)
  const url = safeProviderLoginUrl(projection.login.verification_url ?? projection.login.auth_url ?? "")
  const takesCode = customChoiceId === PROVIDER_LOGIN_CODE_CHOICE_ID
  const userCode = projection.login.user_code
  return {
    title: `Sign in to ${name} · ${accountLabel}`,
    url,
    steps: [
      userCode ? `Enter the code ${userCode} and authorize Chariox.` : "Authorize Chariox in your browser.",
      takesCode ? "Paste the code below and press Enter." : "Chariox continues by itself once you have authorized.",
    ],
    takesCode,
  }
}

/** The provider's latest problem in a login's projected (already redacted)
 * output, e.g. a rejected code or the kernel's failure note; otherwise, when
 * `fallbackToLastLine`, its last non-empty line. */
export function providerLoginProblem(terminalOutputBase64: string, fallbackToLastLine = false): string | null {
  let text: string
  try {
    text = new TextDecoder().decode(Uint8Array.from(atob(terminalOutputBase64), c => c.charCodeAt(0)))
  } catch { return null }
  const lines = text.replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "").replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .split(/\r?\n|\r/).map(line => line.replace(/[\x00-\x1f\x7f]/g, "").trim()).filter(Boolean)
  const problem = [...lines].reverse().find(line => /error|fail|invalid|denied|expired|timed out|not accept|nothing was stored/i.test(line))
  return problem ?? (fallbackToLastLine ? lines.at(-1) ?? null : null)
}

export function safeProviderLoginUrl(value: string): string | null {
  try {
    const url = new URL(value)
    return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password ? url.href : null
  } catch { return null }
}
