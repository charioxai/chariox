import assert from "node:assert/strict"
import test from "node:test"

import { DEFAULT_CONTROL_PLANE_URL } from "./default-control-plane.js"

test("the default connected service is one HTTPS origin without a path", () => {
  const url = new URL(DEFAULT_CONTROL_PLANE_URL)
  assert.equal(url.protocol, "https:")
  assert.equal(url.origin, DEFAULT_CONTROL_PLANE_URL)
})
