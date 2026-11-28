import { statSync } from "node:fs"
import { join, resolve } from "node:path"

// The drill uses CHARIOX_HOME without a state.path override; durable state lives under kernels/<id>.
export const kernelStateSurfaces = home => [
  { label: "kernel-state", path: join(home, "state") },
  { label: "kernel-durable-state", path: join(home, "kernels") },
]

export function kernelBrowserScript(path) {
  if (!path) return undefined
  let entry = resolve(path)
  try {
    if (statSync(entry).isDirectory()) entry = join(entry, "kernel-browser-host.mjs")
    if (!statSync(entry).isFile()) throw new Error("not a file")
  } catch {
    throw new Error("D3_KERNEL_SCRIPT must name an existing kernel-browser-host.mjs or a directory containing it")
  }
  return entry
}
