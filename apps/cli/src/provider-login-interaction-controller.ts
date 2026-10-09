import {
  PROVIDER_LOGIN_CODE_CHOICE_ID,
  providerLoginName,
  providerLoginProblem,
  providerLoginView,
  type ProviderLoginView,
} from "@chariox/kernel-client/provider-login-projection"
import type { ProviderAuthStatus, ProviderLoginStatus, RuntimeInteraction } from "./cli-types.js"
import type { ProviderLoginLinkOptions } from "./provider-login-link.js"

/** The kernel ends a provider login 10 minutes after it starts. */
const PROVIDER_LOGIN_TIMEOUT_MS = 10 * 60_000
const POLL_MS = 1_000
const MAX_POLL_FAILURES = 3

type FooterTone = "info" | "error"
type NoticeTone = "muted" | "warning"

export type ProviderLoginRef = { login_id: string; provider: string; account_profile: string }

export type ProviderLoginStripState = {
  view: ProviderLoginView
  /** When the kernel ends the login, if known. */
  deadlineMs: number | null
  /** A code was sent and the kernel has not answered yet. */
  checking: boolean
  /** What the provider or kernel last reported going wrong. */
  problem: string | null
}

export type ProviderLoginInteractionControllerDeps = {
  getLoginStatus: (loginId: string) => Promise<ProviderLoginStatus>
  getAuthStatus: (provider: string, accountProfile: string) => Promise<ProviderAuthStatus>
  accountLabel: (provider: string, accountProfile: string) => string
  localDesktop: () => boolean
  openUrl: (url: string) => Promise<boolean>
  copyUrl: (url: string) => Promise<string>
  showPlainLink: (url: string, options: ProviderLoginLinkOptions) => Promise<boolean | void>
  /** Hands text pasted in the plain link view to the code field. */
  pasteCode: (interaction: RuntimeInteraction, text: string) => void
  appendNotice: (message: string, tone?: NoticeTone) => void
  flashFooter: (message: string, tone: FooterTone) => void
  render: () => void
  now?: () => number
  setTimer?: (callback: () => void, ms: number) => unknown
  clearTimer?: (timer: unknown) => void
}

type TrackedLogin = {
  ref: ProviderLoginRef
  status: ProviderLoginStatus | null
  timer: unknown
  failures: number
  /** Output and problem when the last code was sent; a new problem answers it. */
  sentOutput: string | null
  sentProblem: string | null
  problem: string | null
}

/** MP-08/MP-11: the TUI side of a kernel-owned provider login. It follows the
 * kernel's own login status, so whichever client sends the code, every
 * attached TUI shows the same progress and the same final result. */
export function createProviderLoginInteractionController(deps: ProviderLoginInteractionControllerDeps) {
  const now = deps.now ?? Date.now
  const setTimer = deps.setTimer ?? ((callback, ms) => setTimeout(callback, ms))
  const clearTimer = deps.clearTimer ?? ((timer) => clearTimeout(timer as ReturnType<typeof setTimeout>))
  const tracked = new Map<string, TrackedLogin>()
  const finished = new Set<string>()

  const subject = (ref: ProviderLoginRef) =>
    `${providerLoginName(ref.provider)} · ${deps.accountLabel(ref.provider, ref.account_profile)}`

  const finish = async (login: TrackedLogin, status: ProviderLoginStatus | null, error?: string) => {
    const { ref } = login
    tracked.delete(ref.login_id)
    finished.add(ref.login_id)
    clearTimer(login.timer)
    const label = deps.accountLabel(ref.provider, ref.account_profile)
    if (status?.state === "succeeded") {
      const auth = await deps.getAuthStatus(ref.provider, ref.account_profile).catch(() => null)
      const message = `Signed in to ${providerLoginName(ref.provider)}${auth?.identity_summary ? ` as ${auth.identity_summary}` : ""} · saved to ${label}`
      deps.appendNotice(message)
      deps.flashFooter(message, "info")
    } else {
      const message = status?.state === "cancelled"
        ? `${subject(ref)}: sign-in cancelled`
        : `${subject(ref)}: sign-in failed — ${error
          ?? providerLoginProblem(status?.terminal_output_base64 ?? "", true)
          ?? "the provider did not report a reason"}`
      deps.appendNotice(`${message}\nRetry: /provider login ${ref.provider} ${label}`, "warning")
      deps.flashFooter(message, "error")
    }
    deps.render()
  }

  const poll = async (login: TrackedLogin) => {
    if (tracked.get(login.ref.login_id) !== login) return
    let status: ProviderLoginStatus
    try {
      status = await deps.getLoginStatus(login.ref.login_id)
      login.failures = 0
    } catch (error) {
      login.failures += 1
      if (login.failures >= MAX_POLL_FAILURES) {
        await finish(login, null, `could not read the result from the kernel (${error instanceof Error ? error.message : String(error)}); check /provider status ${login.ref.provider}`)
        return
      }
      login.timer = setTimer(() => { void poll(login) }, POLL_MS)
      return
    }
    if (tracked.get(login.ref.login_id) !== login) return
    login.status = status
    if (status.state !== "running") {
      await finish(login, status)
      return
    }
    if (login.sentOutput !== null && status.terminal_output_base64 !== login.sentOutput) {
      const problem = providerLoginProblem(status.terminal_output_base64)
      if (problem && problem !== login.sentProblem) {
        login.sentOutput = null
        login.problem = problem
        deps.flashFooter(`${subject(login.ref)}: ${problem}`, "error")
      }
    }
    // Every poll also advances the visible countdown.
    deps.render()
    login.timer = setTimer(() => { void poll(login) }, POLL_MS)
  }

  const track = (ref: ProviderLoginRef) => {
    if (!ref.login_id || tracked.has(ref.login_id) || finished.has(ref.login_id)) return
    const login: TrackedLogin = { ref, status: null, timer: null, failures: 0, sentOutput: null, sentProblem: null, problem: null }
    tracked.set(ref.login_id, login)
    login.timer = setTimer(() => { void poll(login) }, 0)
  }

  const refOf = (interaction: RuntimeInteraction): ProviderLoginRef | null => {
    const login = interaction.provider_login?.login
    return login?.login_id ? { login_id: login.login_id, provider: login.provider, account_profile: login.account_profile } : null
  }

  /** The strip state of a provider login interaction, or null for others. */
  const stripState = (interaction: RuntimeInteraction): ProviderLoginStripState | null => {
    const projection = interaction.provider_login
    if (!projection) return null
    const ref = refOf(interaction)
    if (ref) track(ref)
    const view = providerLoginView(
      projection,
      interaction.custom_choice?.id,
      deps.accountLabel(projection.login.provider, projection.login.account_profile),
    )
    // Retry and Vault phases belong to the kernel. An old authorization URL
    // must not hide their message, choices or passphrase field.
    if (projection.login.login_kind === "terminal_setup_token" && !view.takesCode) return null
    // A code prompt shows its steps before the provider prints the link.
    if (!view.url && !view.takesCode) return null
    const login = ref ? tracked.get(ref.login_id) : undefined
    const startedAt = login?.status?.started_at_ms
    return {
      view,
      deadlineMs: startedAt
        ? startedAt + PROVIDER_LOGIN_TIMEOUT_MS
        : interaction.timeout_sec ? interaction.requested_at_ms + interaction.timeout_sec * 1_000 : null,
      checking: Boolean(login && login.sentOutput !== null),
      problem: login?.problem ?? null,
    }
  }

  return {
    track,
    stripState,

    /** Records that this TUI sent the code; the kernel's status answers it. */
    codeSent(interaction: RuntimeInteraction) {
      const ref = refOf(interaction)
      if (!ref || interaction.custom_choice?.id !== PROVIDER_LOGIN_CODE_CHOICE_ID) return false
      track(ref)
      const login = tracked.get(ref.login_id)
      if (login) {
        login.sentOutput = login.status?.terminal_output_base64 ?? ""
        login.sentProblem = providerLoginProblem(login.sentOutput)
        login.problem = null
      }
      deps.flashFooter(`${subject(ref)}: checking the code…`, "info")
      deps.render()
      return true
    },

    async openLink(interaction: RuntimeInteraction) {
      const state = stripState(interaction)
      if (!state?.view.url) return
      try {
        // Only a local desktop can open a browser for the user. Over SSH the
        // plain view leaves the link to the terminal (OSC 8 or URL detection).
        if (deps.localDesktop() && await deps.openUrl(state.view.url)) {
          deps.flashFooter("Opened the link in your browser", "info")
          return
        }
        await deps.showPlainLink(state.view.url, {
          force: true,
          autoOpen: false,
          title: state.view.title,
          ...(state.view.takesCode
            ? {
              steps: [...state.view.steps.slice(0, -1), "Paste the code here (Cmd-V); Chariox returns with it."],
              onPaste: (text: string) => deps.pasteCode(interaction, text),
            }
            : { steps: state.view.steps }),
        })
      } catch (error) {
        deps.flashFooter(`Could not show the link: ${error instanceof Error ? error.message : String(error)}`, "error")
      }
      deps.render()
    },

    async copyLink(interaction: RuntimeInteraction) {
      const url = stripState(interaction)?.view.url
      if (url) deps.flashFooter(`Link: ${await deps.copyUrl(url)}`, "info")
    },

    dispose() {
      for (const login of tracked.values()) clearTimer(login.timer)
      tracked.clear()
    },
  }
}

export type ProviderLoginInteractionController = ReturnType<typeof createProviderLoginInteractionController>
