import process from "node:process"
import { spawn } from "node:child_process"

export async function openExternalUrl(url: string): Promise<boolean> {
  const command = process.platform === "darwin"
    ? "open"
    : process.platform === "win32"
      ? "cmd"
      : "xdg-open"
  const args = process.platform === "win32" ? ["/c", "start", "", url] : [url]
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
