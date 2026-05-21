#!/usr/bin/env node
import readline from "node:readline"

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity })

function respond(id, ok, result, error = null) {
  process.stdout.write(`${JSON.stringify({ id, ok, result, error })}\n`)
}

for await (const line of rl) {
  if (!line.trim()) continue
  const request = JSON.parse(line)
  if (request.type === "shutdown") break
  if (request.type === "validate") {
    respond(request.id, true, { validated: true })
    continue
  }
  if (request.type !== "call") {
    respond(request.id, false, null, `unsupported request type ${request.type}`)
    continue
  }
  respond(request.id, true, {
    connector: request.connector,
    operation: request.operation,
    arguments: request.arguments ?? {},
    config: request.config ?? {},
    credential_id: request.credential?.id ?? null,
    has_secret: Boolean(request.credential?.secret),
  })
}
