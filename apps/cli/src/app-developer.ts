import { execFile } from "node:child_process"
import { constants } from "node:fs"
import { access, realpath, stat } from "node:fs/promises"
import { isAbsolute, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "@chariox/kernel-client"

const commands = new Set(["create", "keygen", "manifest", "validate", "pack", "inspect"])
const protocolCommands = new Set(["create", "manifest", "validate", "pack"])
const maxOutputBytes = 2 * 1024 * 1024

export type AppDeveloperDeps = {
  locateBinary: () => Promise<string>
  execute: (binary: string, args: readonly string[]) => Promise<{ status: number, stdout: string }>
  write: (text: string) => void
}

export function isAppDeveloperCommand(action: string | undefined): boolean {
  return action !== undefined && commands.has(action)
}

/** No kernel connection, shell, dependency installation or implicit Cargo build. */
export async function runAppDeveloperCommand(
  args: readonly string[],
  deps: AppDeveloperDeps = {
    locateBinary: () => locateAppPackageBinary(),
    execute: executePackageCommand,
    write: (text) => { process.stdout.write(text) },
  },
): Promise<boolean> {
  const action = args[0]
  if (!isAppDeveloperCommand(action)) return false
  if (args.length > 64 || args.some((value) => Buffer.byteLength(value) > 4096 || value.includes("\0"))) {
    throw new Error("App developer command arguments exceed their bounds")
  }
  if (args.includes("--kernel-protocol") || args.some((value) => value.startsWith("--kernel-protocol="))) {
    throw new Error("The Chariox CLI supplies its current protocol; use chariox-app-package directly to select another contract.")
  }
  const command = [...args]
  if (protocolCommands.has(action!) && !args.includes("--help")) {
    command.push("--kernel-protocol", String(LOCAL_DAEMON_PROTOCOL_VERSION))
  }
  const response = await deps.execute(await deps.locateBinary(), command)
  if (Buffer.byteLength(response.stdout) > maxOutputBytes) throw new Error("App package tool exceeded its output limit")
  let value: unknown
  try { value = JSON.parse(response.stdout) } catch { throw new Error("App package tool returned an invalid response") }
  if (!value || typeof value !== "object" || !("ok" in value) || typeof value.ok !== "boolean") {
    throw new Error("App package tool returned an invalid response")
  }
  if (response.status !== 0 || !value.ok) {
    const error = "error" in value && value.error && typeof value.error === "object" ? value.error : undefined
    if (error && "code" in error && typeof error.code === "string" && "message" in error && typeof error.message === "string") {
      throw new Error(`${error.code}: ${error.message}`)
    }
    throw new Error("App package command failed")
  }
  if (!("result" in value)) throw new Error("App package tool returned an invalid response")
  deps.write(`${JSON.stringify(value.result, null, 2)}\n`)
  return true
}

async function executable(path: string): Promise<string | undefined> {
  try {
    const selected = await realpath(path)
    if (!(await stat(selected)).isFile()) return undefined
    await access(selected, constants.X_OK)
    return selected
  } catch { return undefined }
}

export async function locateAppPackageBinary(
  env: NodeJS.ProcessEnv = process.env,
  moduleUrl: string = import.meta.url,
): Promise<string> {
  const explicit = env.CHARIOX_APP_PACKAGE_BIN?.trim()
  if (explicit) {
    if (!isAbsolute(explicit)) throw new Error("CHARIOX_APP_PACKAGE_BIN must be an absolute executable path")
    const selected = await executable(explicit)
    if (!selected) throw new Error("CHARIOX_APP_PACKAGE_BIN is not an executable file")
    return selected
  }
  // A source checkout is derived from this module, never the user's cwd or PATH.
  const repository = fileURLToPath(new URL("../../../", moduleUrl))
  const checkout = await stat(join(repository, "packages/app-package/Cargo.toml")).then((entry) => entry.isFile(), () => false)
  const fromTarget = async (target: string) => {
    for (const profile of ["release", "debug"]) {
      const selected = await executable(join(target, profile, "chariox-app-package"))
      if (selected) return selected
    }
    return undefined
  }
  if (checkout && env.CARGO_TARGET_DIR?.trim()) {
    const selected = await fromTarget(resolve(repository, env.CARGO_TARGET_DIR.trim()))
    if (selected) return selected
  }
  const installed = await executable("/usr/local/bin/chariox-app-package")
  if (installed) return installed
  if (checkout) {
    const selected = await fromTarget(join(repository, "target"))
    if (selected) return selected
  }
  throw new Error("The App developer tool is not installed. Install the matching Chariox release, or build chariox-app-package in the source checkout and set CHARIOX_APP_PACKAGE_BIN to its absolute path.")
}

function executePackageCommand(binary: string, args: readonly string[]): Promise<{ status: number, stdout: string }> {
  return new Promise((resolve, reject) => {
    execFile(binary, [...args], { encoding: "utf8", maxBuffer: maxOutputBytes, timeout: 120_000 }, (error, stdout) => {
      if (error && typeof error.code !== "number") {
        reject(new Error("App package tool could not run or exceeded its output/time limit"))
      } else {
        resolve({ status: error ? Number(error.code) : 0, stdout })
      }
    })
  })
}
