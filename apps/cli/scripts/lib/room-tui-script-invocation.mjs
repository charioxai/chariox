export function roomTuiScriptInvocation(command, args, platform = process.platform) {
  const commandArgs = [command, ...args]
  if (platform === "linux") {
    return {
      command: "script",
      args: ["-q", "-e", "-c", commandArgs.map(shellQuote).join(" "), "/dev/null"],
    }
  }
  return { command: "script", args: ["-q", "/dev/null", ...commandArgs] }
}

function shellQuote(value) {
  if (typeof value !== "string" || value.includes("\0")) {
    throw new Error("TUI command arguments must be strings without NUL bytes")
  }
  return `'${value.replaceAll("'", `'\\''`)}'`
}
