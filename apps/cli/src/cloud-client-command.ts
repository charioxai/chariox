import { offerLocalKernelSetup, startLocalKernelSetup } from "./local-kernel-setup.js"
import { CloudClient } from "./cloud-client.js"
import { openExternalUrl } from "./external-url.js"
import { loadPreferences, relayCloudProfile, saveRelayCloudProfile, type RelayCloudProfile } from "./preferences.js"
import { resolveConfiguredCloudRelayApiUrl } from "./cli-options.js"

export function createCloudClientCommands(options: {
  client: CloudClient; isKernelConnected: () => boolean; apiUrl: () => string
  notice: (message: string) => void; saveProfile: (profile: RelayCloudProfile | null) => Promise<void>
  offerSetup?: (profile: RelayCloudProfile) => Promise<void>; setup?: (profile: RelayCloudProfile) => Promise<void>
  accountId?: () => Promise<string | undefined>; refresh: () => Promise<void>; openUrl?: (url: string) => Promise<boolean>
}) {
  return async (args: string[]): Promise<boolean> => {
    const area = args[0]
    if (area === "setup" && options.setup) {
      const profile = await options.client.profile()
      if (!profile) throw new Error("Sign in first with /cloud login")
      await options.setup(profile)
      await options.refresh()
      return true
    }
    if (area !== "login" && area !== "logout" && (options.isKernelConnected() || ![undefined, "open", "status", "kernels"].includes(area))) return false
    if (area === "logout") {
      await options.client.logout()
      await options.saveProfile(null)
      options.notice("Terminal signed out. Kernels keep running.")
      await options.refresh()
      return true
    }
    if (area === "status") {
      const profile = await options.client.profile()
      options.notice(profile ? `Terminal profile: ${profile.accountSlug}\nclient=${profile.clientId}` : "Terminal is signed out. Run /cloud login.")
      return true
    }
    const expectedAccountId = await options.accountId?.()
    const profile = await options.client.login(options.apiUrl(), async verification => {
      options.notice(`Sign in this terminal.\nurl=${verification.verificationUrl}\ncode=${verification.userCode}`)
      await options.openUrl?.(verification.verificationUrl)
    }, expectedAccountId)
    await options.saveProfile(profile)
    await options.refresh()
    options.notice(`Terminal signed in: ${profile.accountSlug}. Choose a kernel from My kernels.`)
    await options.offerSetup?.(profile)
    return true
  }
}

export async function runCloudClientCommand(argv: string[]): Promise<boolean> {
  if (argv[0] === "login") argv = ["cloud", "login", ...argv.slice(1)]
  if (argv[0] === "setup") argv = ["cloud", "setup", ...argv.slice(1)]
  if (argv[0] !== "cloud" || !["login", "logout", "status", "kernels", "setup"].includes(argv[1] ?? "")) return false
  const preferences = await loadPreferences()
  let apiUrl = resolveConfiguredCloudRelayApiUrl(preferences) ?? "https://staging.chariox.com"
  for (let index=2; index<argv.length; index++) {
    if (argv[index] !== "--api-url" || !argv[index+1]) throw new Error("usage: chariox cloud login|logout|status|kernels [--api-url <url>]")
    apiUrl = argv[++index]!
  }
  const client = new CloudClient()
  try {
    if (argv[1] === "kernels") {
      const targets = await client.directory()
      for (const target of targets) process.stdout.write(`${target.machineAlias ?? target.machineId}\t${target.daemonAlias ?? target.daemonId}\t${target.daemonId}\t${target.status.toLowerCase()}\n`)
    } else {
      const handle = createCloudClientCommands({ offerSetup: offerLocalKernelSetup, setup: profile => startLocalKernelSetup(profile), client, isKernelConnected: () => false, apiUrl: () => apiUrl, notice: message => process.stdout.write(`${message}\n`), saveProfile: saveRelayCloudProfile, accountId: async () => relayCloudProfile(preferences)?.accountId, refresh: async () => {}, openUrl: openExternalUrl })
      await handle([argv[1]!])
    }
    return true
  } finally { client.stop() }
}
