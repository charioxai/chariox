#!/usr/bin/env node

const command = process.argv[2] ?? "inspect-env"

if (command === "inspect-env") {
  const namesArg = process.argv.find((arg) => arg.startsWith("--names="))
  const names = namesArg
    ? namesArg.slice("--names=".length).split(",").filter(Boolean)
    : ["OPENAI_API_KEY", "GITHUB_TOKEN", "DB_PASSWORD", "PATH", "HOME", "TMPDIR"]
  const selected = {}
  for (const name of names) {
    selected[name] = Object.prototype.hasOwnProperty.call(process.env, name)
      ? process.env[name]
      : null
  }
  process.stdout.write(`${JSON.stringify({ env: selected })}\n`)
} else {
  process.stderr.write(`unknown fake-agent command: ${command}\n`)
  process.exit(2)
}
