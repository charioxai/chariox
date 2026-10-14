// MP-08/MP-10/MP-11: absence from the home picker is not account unavailability.
import type { ProviderAccountProfile } from "./cli-types.js"
import { providerAccountDisplayLabel, providerAccountFamily, selectedProviderAccount } from "./waiting-room-provider-accounts.js"

export function createProviderAccountLabelController(deps: {
  profiles(): readonly ProviderAccountProfile[] | undefined
  kernelId(): string | undefined
  readProfile?: (provider: string, reference: string) => Promise<ProviderAccountProfile>
  now?: () => number
  changed?: () => void
}) {
  let inventory: readonly ProviderAccountProfile[] | undefined
  let kernel: string | undefined
  const observations = new Map<string, { label: string; expires: number; pending: boolean }>()
  let generation = 0
  let disposed = false
  const now = deps.now ?? Date.now
  return {
    label(provider: string, reference?: string): string {
      const profiles = deps.profiles()
      const kernelId = deps.kernelId()
      if (inventory !== profiles || kernel !== kernelId) {
        inventory = profiles
        kernel = kernelId
        generation++
        observations.clear()
      }
      if (!reference || reference === "default" || !deps.readProfile) {
        const profile = selectedProviderAccount(profiles, provider, reference)
        return profile ? providerAccountDisplayLabel(profile) : "Account status not checked"
      }
      const family = providerAccountFamily(provider)
      const key = JSON.stringify([family, reference])
      let observation = observations.get(key)
      if (!disposed && (!observation || (!observation.pending && observation.expires <= now()))) {
        // Expired success is not current truth; never preserve a sign-in claim
        // across a failed refresh, kernel switch or changed account inventory.
        observation = { label: "Account status not checked", expires: 0, pending: true }
        observations.set(key, observation)
        const admittedGeneration = generation
        const admittedObservation = observation
        // A lookup refusal can come from an older kernel's home-only namespace,
        // or before a managed replica is materialized. It is not proof that the
        // account cannot run. Definitive auth/capacity comes from profile data.
        void deps.readProfile(family, reference).then(profile => {
          if (profile.provider !== family) {
            throw new Error("account observation does not match the selected profile")
          }
          return providerAccountDisplayLabel(profile)
        }).catch(error => error instanceof Error && /account profile .* is not registered for/i.test(error.message)
          ? "Account status not checked · profile lookup failed"
          : "Account status not checked · refresh needed").then(label => {
          if (disposed || generation !== admittedGeneration || observations.get(key) !== admittedObservation) return
          admittedObservation.label = label
          admittedObservation.expires = now() + 30_000
          admittedObservation.pending = false
          deps.changed?.()
        })
      }
      return observation?.label ?? "Account status not checked"
    },
    dispose() { disposed = true; observations.clear() },
  }
}
