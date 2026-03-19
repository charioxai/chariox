import assert from "node:assert/strict"
import test from "node:test"

import { buildServer } from "./index.js"

test("GET /health returns an ok status payload", async () => {
  const { app } = buildServer()

  try {
    const response = await app.inject({ method: "GET", url: "/health" })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json(), { status: "ok" })
  } finally {
    await app.close()
  }
})
