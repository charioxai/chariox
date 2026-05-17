export type ProviderActivityControllerDeps = {
  setWorking: (working: boolean) => void
  handleProviderActivity: (active: boolean) => void
  updateSessionChrome: () => void
}

export function createProviderActivityController(
  deps: ProviderActivityControllerDeps,
) {
  const apply = (active: boolean) => {
    if (active) {
      deps.setWorking(true)
    }
    deps.handleProviderActivity(active)
    deps.updateSessionChrome()
  }

  return {
    apply,
  }
}
