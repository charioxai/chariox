import assert from "node:assert/strict"
import test from "node:test"

import { assertMatchingHetznerCheckoutCommit } from "./native-tui-remote-execution.mjs"

const LOCAL_COMMIT = "1111111111111111111111111111111111111111"
const REMOTE_COMMIT = "2222222222222222222222222222222222222222"

test("accepts a Hetzner checkout at the home commit", () => {
  assert.doesNotThrow(() => assertMatchingHetznerCheckoutCommit({
    localCommit: `${LOCAL_COMMIT}\n`,
    remoteCommit: LOCAL_COMMIT,
    remoteRepo: "/tmp/arroba-run",
  }))
})

test("rejects a stale Hetzner checkout before running a drill", () => {
  assert.throws(
    () => assertMatchingHetznerCheckoutCommit({
      localCommit: LOCAL_COMMIT,
      remoteCommit: REMOTE_COMMIT,
      remoteRepo: "/tmp/arroba-run",
    }),
    /remote worker checkout `\/tmp\/arroba-run` is at commit 2222.*home checkout expects 1111/,
  )
})

test("rejects unverifiable checkout revisions", () => {
  assert.throws(
    () => assertMatchingHetznerCheckoutCommit({
      localCommit: "not-a-commit",
      remoteCommit: REMOTE_COMMIT,
      remoteRepo: "/tmp/arroba-run",
    }),
    /could not verify local and remote checkout commits/,
  )
})
