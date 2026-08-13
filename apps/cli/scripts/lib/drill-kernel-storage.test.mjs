import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  drillKernelStoragePaths,
  isolatedKernelConfigToml,
} from "./drill-kernel-storage.mjs"

test("drill kernel config isolates all persistent stores", () => {
  const root = path.join("", "tmp", "chariox-drill", "home-storage")
  const paths = drillKernelStoragePaths(root)

  assert.deepEqual(Object.keys(paths).sort(), [
    "operationalArtifactIndexPath",
    "operationalArtifactRoot",
    "operationalHistoryPath",
    "statePath",
  ])
  assert.ok(Object.values(paths).every((value) => value.startsWith(root)))
  const config = isolatedKernelConfigToml(root, [
    "[slices]",
    `root = ${JSON.stringify(path.join(root, "slices"))}`,
  ])
  assert.match(config, /\[state\]/)
  assert.match(config, /\[history\.operational\]/)
  assert.match(config, /\[artifacts\.operational\]/)
  assert.match(config, /\[slices\]/)
  assert.doesNotMatch(config, /~\/\.chariox|\/Users\//)
})
