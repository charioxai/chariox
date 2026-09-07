import assert from "node:assert/strict"
import test from "node:test"

import { buildHostedCloudViewUrl } from "./cloud-command-lifecycle.js"

test("buildHostedCloudViewUrl targets the authenticated Cloud View route", () => {
  assert.equal(
    buildHostedCloudViewUrl("https://cloud.test/api", {
      sessionId: "session one",
      agentId: "agent/two",
      sliceId: "slice three",
    }),
    "https://cloud.test/view?view_target=session+one%3Aagent%2Ftwo%3Aslice+three",
  )
})
