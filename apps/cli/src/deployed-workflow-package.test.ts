import assert from "node:assert/strict"
import { rm, writeFile } from "node:fs/promises"
import { join } from "node:path"
import test from "node:test"

import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import { deployedWorkflowPackageFixture } from "./deployed-workflow-package.test-support.js"

test("deployed workflow package preflight produces deterministic verified digests", async () => {
  const root = await deployedWorkflowPackageFixture()
  try {
    const first = await preparePublicationReleasePackage(root)
    const second = await preparePublicationReleasePackage(join(root, "publication.json"))

    assert.equal(first.packageVersion, 4)
    assert.equal(first.contractVersion, 1)
    assert.equal(first.packageId, first.contract.package_id)
    assert.notEqual(first.packageDigest, first.packageId)
    assert.match(first.packageDigest, /^sha256:[a-f0-9]{64}$/)
    assert.match(first.artifact.sha256, /^sha256:[a-f0-9]{64}$/)
    assert.equal(first.artifact.byteSize, Buffer.from(first.artifact.archiveBase64, "base64").byteLength)
    assert.deepEqual(second, first)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("deployed workflow package preflight rejects contents changed after contract creation", async () => {
  const root = await deployedWorkflowPackageFixture()
  try {
    await writeFile(join(root, "run.sh"), "#!/bin/sh\nexit 1\n")
    await assert.rejects(
      preparePublicationReleasePackage(root),
      /do not match the deployment contract package ID/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
