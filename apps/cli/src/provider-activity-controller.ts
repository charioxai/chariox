import {
  providerActivityRuntimeTransition,
} from "@chariox/kernel-client/session-runtime-transition"

export type ProviderActivityControllerDeps = {
  setWorking: (working: boolean) => void
  handleProviderActivity: (active: boolean) => void
  updateSessionChrome: () => void
}

export function createProviderActivityController(
  deps: ProviderActivityControllerDeps,
) {
  const apply = (active: boolean) => {
    const transition = providerActivityRuntimeTransition(active)
    if (transition.working !== null) {
      deps.setWorking(transition.working)
    }
    deps.handleProviderActivity(transition.providerActivityActive)
    if (transition.shouldUpdateSessionChrome) {
      deps.updateSessionChrome()
    }
  }

  return {
    apply,
  }
}
