import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

import {
  BROWSER_COMPUTER_FUNCTIONAL_EVIDENCE_SCHEMA,
  browserComputerFunctionalCases,
  validateBrowserComputerFunctionalEvidence,
} from "./browser-computer-functional-contract.mjs"
import { validateKnownArtifactContents } from "./drill-artifact-content-validation.mjs"

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..")

test("functional catalog covers browser, computer, shared control, named faults, resources, and cleanup", () => {
  const cases = browserComputerFunctionalCases()
  assert.deepEqual([...new Set(cases.map((item) => item.category))], [
    "browser",
    "computer",
    "shared",
    "fault",
    "resource",
    "cleanup",
  ])
  assert.deepEqual(cases.filter((item) => item.category === "fault").map((item) => item.id), [
    "fault.relay-loss",
    "fault.controller-crash",
    "fault.streamer-crash",
    "fault.browser-crash",
    "fault.stale-endpoint",
    "fault.queue-saturation",
    "fault.memory-pressure",
    "fault.disk-pressure",
  ])
  assert.throws(() => cases.push({}), TypeError)
  assert.throws(() => cases[0].assertions.push("drift"), TypeError)
})

test("functional catalog requires OAuth callback and external reauthentication evidence", () => {
  assert.equal(BROWSER_COMPUTER_FUNCTIONAL_EVIDENCE_SCHEMA, "chariox.browser_computer.functional_evidence.v3")
  const authentication = browserComputerFunctionalCases().find((item) => item.id === "browser.authentication")
  assert.deepEqual(authentication?.assertions, [
    "oauth-popup-callback-completed",
    "service-session-invalidation-distinguished",
    "browser-state-present-during-reauth",
    "reauthentication-restores-use",
  ])
})

test("functional evidence accepts an exact passing subset", () => {
  const caseIds = ["browser.persistence", "computer.observe", "cleanup.resources"]
  const report = evidence(caseIds)
  assert.equal(validateBrowserComputerFunctionalEvidence(report, {
    requiredCaseIds: caseIds,
    repoRoots: ["/work/chariox"],
  }), report)
  assert.equal(report.coverage, "subset")
})

test("functional evidence preserves red cases and derives the failed status", () => {
  const caseIds = ["fault.controller-crash"]
  const report = evidence(caseIds)
  report.cases[0].assertions[1].ok = false
  report.cases[0].assertions[1].evidence = "controller health remained ready after injected death"
  report.cases[0].status = "failed"
  report.status = "failed"
  report.ok = false

  assert.equal(validateBrowserComputerFunctionalEvidence(report, { requiredCaseIds: caseIds }), report)
  assert.throws(
    () => validateBrowserComputerFunctionalEvidence({ ...report, status: "passed", ok: true }, { requiredCaseIds: caseIds }),
    /status must be failed/,
  )
})

test("functional evidence rejects missing coverage, assertion drift, in-repo artifacts, and secrets", () => {
  const caseIds = ["browser.form", "cleanup.resources"]
  const report = evidence(caseIds)
  assert.throws(
    () => validateBrowserComputerFunctionalEvidence({ ...report, cases: report.cases.slice(0, 1) }, { requiredCaseIds: caseIds }),
    /case order or coverage differs/,
  )

  const assertionDrift = structuredClone(report)
  assertionDrift.cases[0].assertions[0].id = "different-assertion"
  assert.throws(
    () => validateBrowserComputerFunctionalEvidence(assertionDrift, { requiredCaseIds: caseIds }),
    /assertions do not match/,
  )

  assert.throws(
    () => validateBrowserComputerFunctionalEvidence({ ...report, artifactRoot: "/work/chariox/.artifacts/run" }, {
      requiredCaseIds: caseIds,
      repoRoots: ["/work/chariox"],
    }),
    /must stay outside repositories/,
  )

  const secret = structuredClone(report)
  secret.cases[0].assertions[0].evidence = "Bearer abcdefghijklmnopqrstuvwxyz"
  assert.throws(
    () => validateBrowserComputerFunctionalEvidence(secret, { requiredCaseIds: caseIds }),
    /secret-looking value/,
  )
})

test("not-applicable cases require a reason and keep the run red", () => {
  const caseIds = ["computer.desktop"]
  const report = evidence(caseIds)
  report.cases[0] = {
    caseId: caseIds[0],
    status: "not_applicable",
    reason: "profile has no graphical desktop",
    assertions: [],
  }
  report.status = "failed"
  report.ok = false
  assert.equal(validateBrowserComputerFunctionalEvidence(report, { requiredCaseIds: caseIds }), report)

  delete report.cases[0].reason
  assert.throws(
    () => validateBrowserComputerFunctionalEvidence(report, { requiredCaseIds: caseIds }),
    /reason must be non-empty text/,
  )
})

test("generic artifact validation recognizes complete and declared subset evidence", () => {
  const caseIds = browserComputerFunctionalCases().map((item) => item.id)
  const report = evidence(caseIds)
  assert.equal(report.coverage, "complete")
  assert.doesNotThrow(() => validateKnownArtifactContents(
    Buffer.from(JSON.stringify(report)),
    "/evidence/browser-computer-functional.json",
  ))

  report.cases[0].assertions[0].ok = false
  assert.throws(
    () => validateKnownArtifactContents(
      Buffer.from(JSON.stringify(report)),
      "/evidence/browser-computer-functional.json",
    ),
    /status does not match its assertions/,
  )

  const subset = evidence(["browser.discovery", "cleanup.resources"])
  assert.doesNotThrow(() => validateKnownArtifactContents(
    Buffer.from(JSON.stringify(subset)),
    "/evidence/browser-computer-functional.json",
  ))
})

test("generic artifact validation rejects repository-local evidence and dishonest scope", () => {
  const report = evidence(["browser.discovery"])
  report.artifactRoot = path.join(REPO_ROOT, ".artifacts", "run")
  assert.throws(
    () => validateKnownArtifactContents(
      Buffer.from(JSON.stringify(report)),
      "/evidence/browser-computer-functional.json",
    ),
    /must stay outside repositories/,
  )

  const dishonestScope = evidence(["browser.discovery"])
  dishonestScope.coverage = "complete"
  assert.throws(
    () => validateBrowserComputerFunctionalEvidence(dishonestScope),
    /coverage must be subset/,
  )

  const undeclaredCase = evidence(["browser.discovery"])
  undeclaredCase.cases.push(evidence(["cleanup.resources"]).cases[0])
  assert.throws(
    () => validateBrowserComputerFunctionalEvidence(undeclaredCase),
    /case order or coverage differs/,
  )
})

function evidence(caseIds) {
  const definitions = new Map(browserComputerFunctionalCases().map((item) => [item.id, item]))
  return {
    schema: BROWSER_COMPUTER_FUNCTIONAL_EVIDENCE_SCHEMA,
    runId: "m0-functional-run",
    startedAt: "2026-08-30T10:00:00.000Z",
    completedAt: "2026-08-30T10:01:00.000Z",
    status: "passed",
    ok: true,
    profile: {
      id: "local-macos-docker-desktop",
      platform: "darwin-arm64",
      topology: "same-host-docker",
    },
    stack: {
      display: "novnc",
      displayVersion: "1.6.0",
      browserControl: "one-shot-cdp",
      browserControlVersion: "9af6d0230",
      browser: "chromium",
      browserVersion: "140.0.0",
    },
    artifactRoot: "/Users/tester/.codex/evidence/browser-computer-use/m0/m0-functional-run",
    caseIds,
    coverage: caseIds.length === browserComputerFunctionalCases().length ? "complete" : "subset",
    cases: caseIds.map((caseId) => ({
      caseId,
      status: "passed",
      assertions: definitions.get(caseId).assertions.map((id) => ({
        id,
        ok: true,
        evidence: `recorded ${id}`,
      })),
    })),
  }
}
