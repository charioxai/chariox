import assert from "node:assert/strict"
import test from "node:test"

import { workflowPublicationPackageDigest } from "./workflow-publication-package-digest.js"

test("workflow trigger package digest matches the kernel and Cloud vector", () => {
  assert.equal(workflowPublicationPackageDigest([
    {
      path: "publication.json",
      content: Buffer.from("{}"),
      executable: false,
    },
    {
      path: "entrypoint.sh",
      content: Buffer.from("#!/bin/sh\nexit 0\n"),
      executable: true,
    },
  ]), "sha256:41adbfede761eb36ea3202865c16f8e3c1f5b232994d16bde44852ebb3687f4a")
})

test("workflow trigger package digest length-frames structural records", () => {
  const left = workflowPublicationPackageDigest([{
    path: "z",
    content: Buffer.from("b"),
    executable: false,
  }])
  const right = workflowPublicationPackageDigest([{
    path: "zYg==",
    content: Buffer.alloc(0),
    executable: false,
  }])

  assert.equal(left, "sha256:586e9150a9d5fb2f9acdfda4a029feb0ea7aaa1c91332d142c1b5e305051f7e5")
  assert.equal(right, "sha256:cf85ff5ab71aacd4309f63acea96197342afc4cf6e9af3eb955ddaf73a90a53c")
  assert.notEqual(left, right)
})
