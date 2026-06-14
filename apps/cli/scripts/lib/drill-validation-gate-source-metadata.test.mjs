import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import { validationGateEvidenceSourceMetadata } from "./drill-validation-gate-source-metadata.mjs"

test("builds validation gate evidence source metadata by repo root", () => {
  const root = path.join(path.sep, "tmp", "arroba-validation-sources")
  const ossRoot = path.join(root, "arroba")
  const cloudRoot = path.join(root, "arroba-cloud")

  const metadata = validationGateEvidenceSourceMetadata({
    checks: {
      matrices: {
        aggregate: {
          reports: [
            { source: path.join(ossRoot, ".artifacts", "matrices", "native.json") },
            { source: path.join(cloudRoot, ".artifacts", "matrices", "slice.json") },
            { source: path.join(root, "external", "matrix.json") },
          ],
        },
      },
      artifacts: {
        aggregate: {
          indexes: [
            {
              source: path.join(ossRoot, ".artifacts", "suite", "arroba-drill-artifacts.json"),
              rootDir: path.join(ossRoot, ".artifacts", "suite"),
            },
            {
              source: path.join(cloudRoot, ".artifacts", "suite", "arroba-drill-artifacts.json"),
              rootDir: path.join(cloudRoot, ".artifacts", "suite"),
            },
          ],
        },
      },
      failures: {
        aggregate: {
          failures: [
            {
              source: path.join(ossRoot, ".artifacts", "failed", "arroba-drill-failure.json"),
              rootDir: path.join(ossRoot, ".artifacts", "failed"),
            },
          ],
        },
      },
    },
  }, { ossRoot, cloudRoot })

  assert.deepEqual(metadata, {
    artifactEvidenceRepos: "cloud,oss",
    evidenceRepos: "cloud,external,oss",
    failureEvidenceRepos: "oss",
    matrixEvidenceRepos: "cloud,external,oss",
  })
})

test("does not classify sibling paths as repo evidence", () => {
  const root = path.join(path.sep, "tmp", "arroba-validation-sources")
  const ossRoot = path.join(root, "arroba")

  const metadata = validationGateEvidenceSourceMetadata({
    checks: {
      matrices: {
        aggregate: {
          reports: [
            { source: path.join(`${ossRoot}-backup`, ".artifacts", "matrix.json") },
          ],
        },
      },
    },
  }, { ossRoot })

  assert.deepEqual(metadata, {
    evidenceRepos: "external",
    matrixEvidenceRepos: "external",
  })
})

test("omits empty source metadata when there is no evidence", () => {
  assert.deepEqual(validationGateEvidenceSourceMetadata({ checks: {} }), {})
})
