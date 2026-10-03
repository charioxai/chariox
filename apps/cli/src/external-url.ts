import process from "node:process"
import { spawn } from "node:child_process"

export async function openExternalUrl(url: string): Promise<boolean> {
  const command = process.platform === "darwin"
    ? "open"
    : process.platform === "win32"
      ? "rundll32.exe"
      : "xdg-open"
  const args = process.platform === "win32" ? ["url.dll,FileProtocolHandler", url] : [url]
  return await new Promise((resolve) => {
    const child = spawn(command, args, {
      detached: true,
      stdio: "ignore",
    })
    child.once("error", () => resolve(false))
    child.once("spawn", () => {
      child.unref()
      resolve(true)
    })
  })
}
