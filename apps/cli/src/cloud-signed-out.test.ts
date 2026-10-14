import assert from "node:assert/strict"
import test from "node:test"
import { CLOUD_SIGNED_OUT_NOTICE, presentCloudSignedOut } from "./cloud-signed-out.js"

test("BY-03: a signed-out terminal drops its profile and directory and shows one sign-in-again notice", () => {
  const calls: string[] = []
  presentCloudSignedOut({ clearProfile: () => calls.push("profile"), clearDirectory: () => calls.push("directory"), notify: message => calls.push(message) })
  assert.deepEqual(calls, ["profile", "directory", CLOUD_SIGNED_OUT_NOTICE])
  assert.match(CLOUD_SIGNED_OUT_NOTICE, /^Signed out\. Sign in again/)
  assert.match(CLOUD_SIGNED_OUT_NOTICE, /\/cloud login/)
})
