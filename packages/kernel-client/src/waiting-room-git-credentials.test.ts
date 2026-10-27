import assert from "node:assert/strict"
import test from "node:test"

import type { WaitingRoomInventorySnapshot } from "./kernel-types-cloud.js"

test("Waiting Room inventory preserves the versioned Git credential summary", () => {
  const snapshot = {
    inventory_version: "inventory-1",
    structural_version: "structural-1",
    activity_revision: "activity-1",
    sessions: [],
    projects: [],
    relay_status: {
      configured: true,
      connected: true,
      relay_token_configured: true,
      daemon_id: "kernel-1",
      machine_id: "machine-1",
    },
    git_credentials: [{
      credentialId: "github",
      hostname: "github.com",
      label: "GitHub",
    }],
  } satisfies WaitingRoomInventorySnapshot

  assert.deepEqual(snapshot.git_credentials, [{
    credentialId: "github",
    hostname: "github.com",
    label: "GitHub",
  }])
})
