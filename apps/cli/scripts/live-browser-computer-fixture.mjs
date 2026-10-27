#!/usr/bin/env node
import { startBrowserComputerFixture } from "./lib/browser-computer-fixture.mjs"

const password = process.env.CHARIOX_BROWSER_FIXTURE_PASSWORD
if (!password) {
  console.error("CHARIOX_BROWSER_FIXTURE_PASSWORD is required")
  process.exit(1)
}

const fixture = await startBrowserComputerFixture({
  account: process.env.CHARIOX_BROWSER_FIXTURE_ACCOUNT ?? "agent@chariox.test",
  host: process.env.CHARIOX_BROWSER_FIXTURE_HOST ?? "127.0.0.1",
  port: parsePort(process.env.CHARIOX_BROWSER_FIXTURE_PORT),
  password,
})

console.log(`CHARIOX_BROWSER_COMPUTER_FIXTURE_READY ${JSON.stringify({
  account: fixture.account,
  origin: fixture.origin,
})}`)

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, async () => {
    await fixture.close()
    process.exit(0)
  })
}

function parsePort(value) {
  if (value === undefined || value === "") return 0
  const port = Number(value)
  if (!Number.isInteger(port) || port < 0 || port > 65_535) {
    throw new Error("CHARIOX_BROWSER_FIXTURE_PORT must be a valid TCP port")
  }
  return port
}
