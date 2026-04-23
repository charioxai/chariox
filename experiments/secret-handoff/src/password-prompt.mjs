#!/usr/bin/env node

const expected = process.argv[2]
if (!expected) {
  process.stderr.write("usage: password-prompt.mjs <expected-password>\n")
  process.exit(2)
}

process.stdout.write("Password:")

let input = ""
process.stdin.setEncoding("utf8")
process.stdin.on("data", (chunk) => {
  input += chunk
  if (!input.includes("\n")) return
  const received = input.trimEnd()
  if (received === expected) {
    process.stdout.write("\nAUTH_OK\n")
    process.exit(0)
  }
  process.stdout.write("\nAUTH_FAILED\n")
  process.exit(1)
})
