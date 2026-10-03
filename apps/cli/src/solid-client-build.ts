import { createEffect, createRoot, createSignal } from "solid-js"

// The TUI's key handlers, mount hooks and live views all depend on effects,
// which Solid's server build silently drops.
export function assertSolidClientBuild() {
  let observed = 0
  const [setValue, dispose] = createRoot((dispose) => {
    const [value, setValue] = createSignal(1)
    createEffect(() => { observed = value() })
    return [setValue, dispose] as const
  })
  setValue(2)
  dispose()
  if (observed !== 2) {
    throw new Error("solid-js resolved to its server build; start the CLI through dist/index.js")
  }
}
