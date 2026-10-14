import type { ProviderAccountProfile } from "@chariox/kernel-client"
import { providerAccountFamily, providerAccountsForProvider } from "./waiting-room-provider-accounts.js"

/**
 * MP-08: the waiting-room Provider Accounts popup. It offers the same account
 * actions as `/provider accounts` and the web panel; every action runs (or,
 * when it needs typed input or confirmation, stages) the existing command.
 */
export type ProviderAccountsPopupCommand =
  | { readonly kind: "run"; readonly command: string }
  | { readonly kind: "stage"; readonly command: string }
  | { readonly kind: "use"; readonly provider: string; readonly profileId: string }

export type ProviderAccountsPopupChoice = {
  readonly label: string
  readonly command: ProviderAccountsPopupCommand
}

export type ProviderAccountsPopupItem = {
  readonly label: string
  readonly choices: readonly ProviderAccountsPopupChoice[]
}

const PROVIDER_FAMILIES = ["claude", "codex", "opencode"] as const

export function providerFamilyLabel(provider: string): string {
  const family = providerAccountFamily(provider)
  return family === "claude" ? "Claude" : family === "codex" ? "Codex" : family === "opencode" ? "OpenCode" : family
}

/** Account families offered on this kernel; an older kernel reports none and keeps all. */
export function providerAccountsPopupFamilies(installedProviders: readonly string[] | undefined): readonly string[] {
  return installedProviders
    ? PROVIDER_FAMILIES.filter((family) => installedProviders.includes(family))
    : PROVIDER_FAMILIES
}

const ADD_METHODS: Readonly<Record<string, readonly { readonly label: string; readonly method: string }[]>> = {
  claude: [{ label: "Claude sign-in link", method: "terminal" }],
  codex: [{ label: "Device code link", method: "device_code" }],
  opencode: [
    { label: "OpenCode Go", method: "opencode_go_api_key" },
    { label: "OpenCode Zen", method: "opencode_zen_api_key" },
    { label: "Other OpenCode provider", method: "terminal" },
  ],
}

export function providerAccountAddChoices(provider: string): readonly ProviderAccountsPopupChoice[] {
  const family = providerAccountFamily(provider)
  return (ADD_METHODS[family] ?? []).map(({ label, method }) => ({
    label,
    command: { kind: "run", command: `/provider accounts add ${family} --method ${method}` },
  }))
}

export function providerAccountsPopupItems(options: {
  readonly profiles: readonly ProviderAccountProfile[] | undefined
  readonly installedProviders: readonly string[] | undefined
  readonly providerId: string
}): readonly ProviderAccountsPopupItem[] {
  const selected = providerAccountFamily(options.providerId)
  const families = [...providerAccountsPopupFamilies(options.installedProviders)]
    .sort((left, right) => Number(right === selected) - Number(left === selected))
  return [
    ...families.flatMap((family) => [
      ...providerAccountsForProvider(options.profiles, family).map((profile) => accountItem(profile)),
      { label: `Add new ${providerFamilyLabel(family)} account`, choices: providerAccountAddChoices(family) },
    ]),
    ...(families.length ? [{
      label: "Link existing provider directory",
      choices: families.map((family): ProviderAccountsPopupChoice => ({
        label: providerFamilyLabel(family),
        command: { kind: "stage", command: `/provider accounts link ${family} ` },
      })),
    }] : []),
  ]
}

function accountItem(profile: ProviderAccountProfile): ProviderAccountsPopupItem {
  const ref = `${profile.provider} ${profile.profile_id}`
  const signedIn = profile.auth_state === "authenticated"
  const state = signedIn ? "signed in" : profile.auth_state === "unknown" ? "sign-in not checked" : "needs sign-in"
  const choices: ProviderAccountsPopupChoice[] = [
    ...(signedIn ? [] : [{ label: "Sign in", command: { kind: "run", command: `/provider login ${ref}` } } as const]),
    { label: "Use for new session", command: { kind: "use", provider: profile.provider, profileId: profile.profile_id } },
    { label: "Refresh", command: { kind: "run", command: `/provider accounts refresh ${ref}` } },
    ...(profile.is_default ? [] : [{ label: "Make default", command: { kind: "run", command: `/provider accounts default ${ref}` } } as const]),
    ...(profile.provider === "claude"
      ? [{ label: "Create setup token", command: { kind: "run", command: `/provider setup-token claude ${profile.profile_id} --run` } } as const]
      : []),
    { label: "Rename", command: { kind: "stage", command: `/provider accounts rename ${ref} ` } },
    ...(signedIn ? [{ label: "Log out", command: { kind: "stage", command: `/provider logout ${ref}` } } as const] : []),
    { label: "Remove", command: { kind: "stage", command: `/provider accounts remove ${ref}` } },
    ...(profile.origin === "chariox_created"
      ? [{ label: "Delete data", command: { kind: "stage", command: `/provider accounts delete ${ref} ${profile.profile_id}` } } as const]
      : []),
  ]
  return {
    label: `${providerFamilyLabel(profile.provider)} · ${profile.label}${profile.is_default ? " (default)" : ""} · ${state}`,
    choices,
  }
}

/** MP-08: Enter on the Account row opens the popup when no account of the provider is signed in. */
export function accountRowOpensProviderAccounts(
  profiles: readonly ProviderAccountProfile[] | undefined,
  providerId: string,
): boolean {
  return profiles !== undefined
    && !providerAccountsForProvider(profiles, providerId).some((profile) => profile.auth_state === "authenticated")
}
