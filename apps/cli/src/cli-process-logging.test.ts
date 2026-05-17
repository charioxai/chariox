import { strict as assert } from "node:assert"
import test from "node:test"

import type { ArrobaLogger } from "./logging.js"
import {
  createCliProcessLoggerRegistry,
  formatCliError,
} from "./cli-process-logging.js"

test("CLI process logger registry returns child loggers after initialization", () => {
  const children: Array<{ component: string, fields: Record<string, unknown> }> = []
  const rootLogger = {
    child(component: string, fields: Record<string, unknown>) {
      children.push({ component, fields })
      return rootLogger
    },
  } as unknown as ArrobaLogger
  const registry = createCliProcessLoggerRegistry({
    createLogger: () => rootLogger,
  })

  assert.equal(registry.getLogger("cli.main"), null)
  assert.equal(registry.initialize("cli"), rootLogger)
  assert.equal(registry.getLogger("cli.main", { argv: ["--help"] }), rootLogger)
  assert.deepEqual(children, [
    { component: "cli.main", fields: { argv: ["--help"] } },
  ])
})

test("formatCliError delegates CLI error presentation", () => {
  assert.equal(formatCliError(new Error("boom")), "boom")
  assert.equal(formatCliError("plain"), "plain")
})
