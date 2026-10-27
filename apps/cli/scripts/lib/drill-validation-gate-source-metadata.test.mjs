import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import { validationGateEvidenceSourceMetadata } from "./drill-validation-gate-source-metadata.mjs"

test("builds validation gate evidence source metadata by repo root", () => {
  const root = path.join(path.sep, "tmp", "chariox-validation-sources")
  const ossRoot = path.join(root, "chariox")
  const cloudRoot = path.join(root, "chariox-cloud")

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
              source: path.join(ossRoot, ".artifacts", "suite", "chariox-drill-artifacts.json"),
              rootDir: path.join(ossRoot, ".artifacts", "suite"),
            },
            {
              source: path.join(cloudRoot, ".artifacts", "suite", "chariox-drill-artifacts.json"),
              rootDir: path.join(cloudRoot, ".artifacts", "suite"),
            },
          ],
        },
      },
      failures: {
        aggregate: {
          failures: [
            {
              source: path.join(ossRoot, ".artifacts", "failed", "chariox-drill-failure.json"),
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
  const root = path.join(path.sep, "tmp", "chariox-validation-sources")
  const ossRoot = path.join(root, "chariox")

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

test("classifies generated evidence paths by repo root", () => {
  const root = path.join(path.sep, "tmp", "chariox-validation-sources")
  const ossRoot = path.join(root, "chariox")
  const cloudRoot = path.join(root, "chariox-cloud")

  const metadata = validationGateEvidenceSourceMetadata({
    checks: {},
    generatedEvidence: {
      validationSuites: {
        outputRoots: [path.join(ossRoot, ".artifacts", "suite")],
        artifactIndexes: [path.join(ossRoot, ".artifacts", "suite", "chariox-drill-artifacts.json")],
        failureRoots: [path.join(root, "external", "suite-failures")],
        commands: [{
          reportPath: path.join(ossRoot, ".artifacts", "suite", "drill-validation-suite-run.json"),
          artifactIndexPath: path.join(ossRoot, ".artifacts", "suite", "chariox-drill-artifacts.json"),
          failureRoot: path.join(root, "external", "suite-failures"),
          scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
        }],
      },
      matrixReports: {
        reportPaths: [path.join(cloudRoot, ".artifacts", "matrices", "cloud-slice.json")],
        artifactIndexes: [path.join(cloudRoot, ".artifacts", "matrices", "chariox-drill-artifacts.json")],
        commands: [{
          reportPath: path.join(cloudRoot, ".artifacts", "matrices", "cloud-slice.json"),
          artifactIndexPath: path.join(cloudRoot, ".artifacts", "matrices", "chariox-drill-artifacts.json"),
          scriptPath: path.join(cloudRoot, "scripts", "cloud-slice-runtime-matrix.mjs"),
        }],
      },
    },
  }, { ossRoot, cloudRoot })

  assert.deepEqual(metadata, {
    evidenceRepos: "cloud,external,oss",
    generatedEvidenceRepos: "cloud,external,oss",
  })
})

test("omits empty source metadata when there is no evidence", () => {
  assert.deepEqual(validationGateEvidenceSourceMetadata({ checks: {} }), {})
})
