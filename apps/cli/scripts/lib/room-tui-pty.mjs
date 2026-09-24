import assert from "node:assert/strict"

export function roomTuiPtyInvocation(commandArgs, platform = process.platform) {
  assert.ok(commandArgs.length > 0 && commandArgs.every(arg => typeof arg === "string" && !arg.includes("\0")), "valid TUI command required")
  if (platform === "darwin") return { command: "script", args: ["-q", "/dev/null", ...commandArgs] }
  if (platform === "linux") {
    const quotedCommand = commandArgs.map(arg => `'${arg.replaceAll("'", "'\\''")}'`).join(" ")
    return { command: "script", args: ["--quiet", "--return", "--command", quotedCommand, "/dev/null"] }
  }
  throw new Error(`Room TUI drill is unsupported on ${platform}`)
}
