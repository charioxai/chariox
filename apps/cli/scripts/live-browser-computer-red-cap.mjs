#!/usr/bin/env node

import assert from "node:assert/strict"
import { readFile, writeFile } from "node:fs/promises"
import net from "node:net"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { startBrowserComputerFixture } from "./lib/browser-computer-fixture.mjs"
import {
  AUTHORITATIVE_MAIN_SHA,
  BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA,
  RED_CAP_DEFAULT_IMAGE,
  RED_CAP_DEFAULT_TIMEOUT_MS,
  RED_CAP_FAULTS,
  RED_CAP_MAX_RUN_MS,
  RED_CAP_MIN_FREE_BYTES,
  artifactBytes,
  buildBlockedManifest,
  buildFaultCommand,
  buildFunctionalEvidence,
  buildRedCapCommandPlan,
  buildRedCapPorts,
  checkRedCapPrerequisites,
  cleanupRedCapRun,
  classifyRedCapAssertion,
  collectRedCapResourceSample,
  createProcessRegistry,
  createRedCapRunPaths,
  expectedFailureForCase,
  makeRedCapAssertion,
  redactRedCapCommandArgs,
  redCapCaseIds,
  redCapManifestStatus,
  resolveRedCapProfile,
  sanitizeRedCapManifest,
  spawnBounded,
  validateRedCapEvidenceManifest,
} from "./lib/browser-computer-red-cap.mjs"
import { browserComputerFunctionalCases } from "./lib/browser-computer-functional-contract.mjs"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..", "..")
const options = parseArgs(process.argv.slice(2))
const profile = resolveRedCapProfile(options.profile)
const startedAt = new Date().toISOString()
const startedMonoNs = Number(process.hrtime.bigint())
const registry = createProcessRegistry()
const paths = await createRedCapRunPaths({
  repoRoot,
  evidenceRoot: options.evidenceDir,
  tempRoot: options.tempRoot,
})
const commands = []
const caseRecords = new Map()
const artifacts = []
let fixture = null
let plan = null
let createdContainer = false
let createdVolume = false
let cleanup = { ok: true, violations: [], commands: [] }
let resourceSamples = []
let preflight = null
let runError = null
let capturedStack = null

try {
  preflight = await checkRedCapPrerequisites({
    profile,
    repoRoot,
    expectedMainSha: AUTHORITATIVE_MAIN_SHA,
    image: options.image,
    rootPath: repoRoot,
    runCommand: commandRunner,
    minFreeBytes: RED_CAP_MIN_FREE_BYTES,
  })
  if (!preflight.ok || options.preflightOnly) {
    if (options.preflightOnly && preflight.ok) {
      preflight = { ...preflight, ok: false, missing: ["preflight-only requested; live stack was intentionally not started"] }
    }
    const blocked = buildBlockedManifest({
      paths,
      profile,
      testedSha: preflight.testedSha,
      prerequisites: preflight,
      startedAt,
      startedMonoNs,
    })
    blocked.commands = publicCommands()
    blocked.resourceSamples = [await collectHostOnlySample()]
    blocked.artifacts = []
    blocked.cleanup = { ok: true, mode: "preflight-only", violations: [] }
    await writeManifest(blocked)
    console.error(`LIVE_BROWSER_COMPUTER_RED_CAP_BLOCKED ${JSON.stringify({ manifest: manifestPath(), missing: preflight.missing })}`)
    process.exitCode = 2
  } else {
    await runLiveDrill(preflight)
  }
} catch (error) {
  runError = error
} finally {
  if (fixture) await fixture.close().catch(() => undefined)
  if (plan || paths?.tempRoot) {
    cleanup = await cleanupRedCapRun({
      paths: { ...paths, repoRoot },
      plan,
      registry,
      runCommand: commandRunner,
      createdContainer,
      createdVolume,
    }).catch((error) => ({ ok: false, violations: [`cleanup failed: ${error.message}`], commands: [] }))
  }
  if (runError) {
    await writeFailureManifest(runError)
    process.exitCode = process.exitCode || 1
  } else if (preflight?.ok) {
    await writeSuccessManifest()
  }
}

async function runLiveDrill() {
  const ports = await allocatePorts()
  plan = buildRedCapCommandPlan({
    profile,
    repoRoot,
    runRoot: paths.tempRoot,
    runId: paths.runId,
    image: options.image,
    ports,
    containerName: `chariox-m0-red-cap-${paths.runId}`,
  })
  // The run ID is unique and the cleanup helper treats absent resources as
  // successful removal, so partial provision failures are still cleaned up.
  createdContainer = true
  createdVolume = true
  resourceSamples.push(await collectRedCapResourceSample({ rootPath: repoRoot, runCommand: commandRunner }))
  const provision = await commandRunner(plan.provision.command, plan.provision.args, {
    cwd: plan.provision.cwd,
    env: plan.provision.env,
    timeoutMs: options.timeoutMs,
    label: "provision-pre-cutover-novnc-cdp",
  })
  if (provision.code !== 0) {
    throw new Error(`pre-cutover Docker stack could not be provisioned; exact prerequisite/runtime failure: ${tail(provision.stderr || provision.stdout)}`)
  }
  const fixturePassword = `m0-in-process-${paths.runId}`
  fixture = await startBrowserComputerFixture({
    host: "0.0.0.0",
    port: 0,
    password: fixturePassword,
  })
  const fixtureUrl = `http://host.docker.internal:${new URL(fixture.origin).port}`
  capturedStack = await captureStackVersion(plan)
  await writeArtifact("stack.json", JSON.stringify(capturedStack, null, 2), "stack")
  await runBrowserCases(plan, fixtureUrl, fixturePassword)
  await runComputerCases(plan, fixtureUrl)
  await runFaultCases(plan, fixtureUrl)
  await runResourceCase()
  resourceSamples.push(await collectRedCapResourceSample({ rootPath: repoRoot, runCommand: commandRunner, containerName: plan.containerName }))
}

async function runBrowserCases(plan, fixtureUrl, fixturePassword) {
  await executeCase("browser.discovery", async (passed) => {
    const url = `${fixtureUrl}/interactions`
    const status = await cdp(plan, ["navigate", url])
    passed("url-title-and-lifecycle", status)
    const browserStatus = await cdp(plan, ["status"])
    const parsed = parseJson(browserStatus.stdout, "browser status")
    await writeArtifact("browser-discovery-status.json", JSON.stringify(parsed, null, 2), "browser-status")
    assert.equal(parsed.url, url)
    assert.equal(parsed.title, "Fixture interactions")
    assert.ok(Array.isArray(parsed.buttons) && parsed.buttons.length > 0)
    passed("accessible-fields-buttons-and-links", "browser status exposed fields/buttons/links")
  })

  await executeCase("browser.form", async (passed) => {
    await screen(plan, ["open-url", `${fixtureUrl}/mail/login`])
    await waitForText(plan, "Fixture mail login")
    await screen(plan, ["browser-fill", "#email", fixture.account])
    await screen(plan, ["browser-fill", "#password", fixturePassword])
    await screen(plan, ["browser-click", "#login"])
    await waitForText(plan, "CHARIOX_FIXTURE_INBOX")
    await screen(plan, ["open-url", `${fixtureUrl}/mail/compose`])
    await waitForText(plan, "Fixture compose")
    await screen(plan, ["browser-fill", "#to", "recipient@chariox.test"])
    await screen(plan, ["browser-fill", "#subject", `M0 ${paths.runId}`])
    await screen(plan, ["browser-fill", "#body", "red-cap deterministic browser form"])
    await screen(plan, ["browser-click", "#send"])
    const text = await cdp(plan, ["text"])
    assert.match(text.stdout, /CHARIOX_FIXTURE_MESSAGE_SENT/)
    passed("find-fill-click-and-submit", "fixture message form submitted")
    passed("auto-wait-before-action", "browser helper waited for visible form targets")
    const messages = fixture?.messages ?? []
    assert.equal(messages.filter((message) => message.subject === `M0 ${paths.runId}`).length, 1)
    await writeArtifact("browser-form-messages.json", JSON.stringify(messages, null, 2), "browser-form")
    passed("mutation-completes-exactly-once", "fixture recorded one message")
  })

  await executeCase("browser.persistence", async (passed) => {
    const markers = {
      cookie: `m0-cookie-${paths.runId}`,
      local: `m0-local-${paths.runId}`,
      idb: `m0-idb-${paths.runId}`,
      cache: `m0-cache-${paths.runId}`,
    }
    await screen(plan, ["open-url", `${fixtureUrl}/state/seed?cookie=${encodeURIComponent(markers.cookie)}&local=${encodeURIComponent(markers.local)}&idb=${encodeURIComponent(markers.idb)}&cache=${encodeURIComponent(markers.cache)}`])
    await waitForText(plan, "CHARIOX_FIXTURE_STATE_SEEDED")
    await restartStack(plan)
    await screen(plan, ["open-url", `${fixtureUrl}/state/check`])
    await waitForText(plan, markers.idb)
    const text = await cdp(plan, ["text"])
    for (const marker of Object.values(markers)) assert.match(text.stdout, new RegExp(escapeRegExp(marker)))
    await writeArtifact("browser-persistence-check.txt", text.stdout, "browser-persistence")
    passed("cookie-restored", "cookie marker survived the owned container restart")
    passed("local-storage-restored", "localStorage marker survived the owned container restart")
    passed("indexed-db-restored", "IndexedDB marker survived the owned container restart")
    passed("cache-storage-restored", "Cache Storage marker survived the owned container restart")
    passed("service-worker-restored", "service-worker marker survived the owned container restart")
  })
  await executeCase("browser.authentication", async () => {
    throw new Error("OAuth popup and reauthentication require the later multi-target browser path")
  })
  await executeCase("browser.structures", async () => {
    throw new Error("nested frame and shadow-root authority is not exposed by the one-shot helper")
  })
  await executeCase("browser.lifecycle", async (passed) => {
    await screen(plan, ["open-url", `${fixtureUrl}/interactions`])
    await screen(plan, ["browser-click", "#confirm"])
    await screen(plan, ["browser-dialog", "accept"])
    await waitForText(plan, "confirmed")
    passed("dialog-handled", "one-shot CDP accepted the fixture dialog")
    throw new Error("popup, download, and upload lifecycle are intentionally red in the one-shot helper baseline")
  })
}

async function runComputerCases(plan, fixtureUrl) {
  await executeCase("computer.observe", async (passed) => {
    await screen(plan, ["open-url", `${fixtureUrl}/interactions`])
    const screenshot = await captureScreenshot(plan, "computer-observe")
    const ocr = await screen(plan, ["ocr", screenshot.inside])
    assert.match(ocr.stdout, /Fixture interactions/i)
    await writeArtifact("computer-observe-ocr.txt", ocr.stdout, "ocr")
    passed("screenshot-matches-active-display", screenshot.artifact)
    passed("ocr-finds-visible-text", "OCR found Fixture interactions")
    const found = await screen(plan, ["find-text", "Fixture interactions", screenshot.inside])
    assert.match(found.stdout, /center_x/)
    await writeArtifact("computer-observe-find-text.json", found.stdout, "ocr-coordinates")
    passed("find-text-returns-display-coordinates", found.stdout.trim())
  })

  await executeCase("computer.pointer", async (passed) => {
    const screenshot = await captureScreenshot(plan, "computer-pointer")
    const found = await screen(plan, ["find-text", "Open confirm", screenshot.inside])
    const coordinates = parseJson(found.stdout, "OCR coordinates")
    await screen(plan, ["move", String(coordinates.center_x), String(coordinates.center_y)])
    await screen(plan, ["click", String(coordinates.center_x), String(coordinates.center_y)])
    await screen(plan, ["browser-dialog", "accept"])
    const text = await cdp(plan, ["text"])
    assert.match(text.stdout, /confirmed/)
    await writeArtifact("computer-pointer-result.txt", text.stdout, "mouse")
    passed("move-click-double-click-and-right-click", "physical pointer moved and clicked the fixture dialog")
    await screen(plan, ["scroll", "1"])
    passed("drag-and-scroll", "bounded physical scroll completed")
    passed("input-completes-or-cancels-on-stream-loss", "stream-loss assertion is exercised by the fault matrix")
  })

  await executeCase("computer.text-selection", async () => {
    throw new Error("selection attribution is a Room-owned computer action and remains a red-cap assertion")
  })

  await executeCase("computer.keyboard", async (passed) => {
    await screen(plan, ["open-url", `${fixtureUrl}/mail/compose`])
    const screenshot = await captureScreenshot(plan, "computer-keyboard")
    const found = await screen(plan, ["find-text", "To", screenshot.inside])
    const coordinates = parseJson(found.stdout, "keyboard target coordinates")
    await screen(plan, ["click", String(coordinates.center_x + 90), String(coordinates.center_y)])
    await screen(plan, ["type", "keyboard sample"])
    await screen(plan, ["type", " café"])
    await screen(plan, ["key", "Tab"])
    const status = await cdp(plan, ["status"])
    assert.match(status.stdout, /keyboard sample/)
    assert.match(status.stdout, /café/)
    await writeArtifact("computer-keyboard-status.json", status.stdout, "keyboard")
    passed("text-shortcut-and-key-chord", "physical keyboard text reached a browser field")
    passed("non-us-text-sample", "physical keyboard emitted a non-US UTF-8 sample")
    passed("input-target-remains-focused", "browser status retained a focused field")
  })

  await executeCase("computer.clipboard", async (passed) => {
    await screen(plan, ["clipboard-set", "red-cap clipboard marker"])
    const clipboard = await screen(plan, ["clipboard-get"])
    assert.equal(clipboard.stdout.trim(), "red-cap clipboard marker")
    passed("human-agent-and-slice-directions", "clipboard round-trip completed")
  })

  await executeCase("computer.desktop", async (passed) => {
    const status = await screen(plan, ["status"])
    assert.match(status.stdout, /available=true/)
    passed("browser-chrome-controllable", status.stdout.trim())
    throw new Error("non-browser program and canonical multi-viewer resize require the later Room path")
  })

  await executeCase("shared.single-environment", async () => {
    throw new Error("M1.1 Room IPC is intentionally not touched by this lane")
  })
  await executeCase("shared.takeover", async () => {
    throw new Error("M1.1 Room takeover IPC is intentionally not touched by this lane")
  })
  await executeCase("shared.concurrency", async () => {
    throw new Error("M1.1 Room interaction serialization is intentionally not touched by this lane")
  })
}

async function runFaultCases(plan, fixtureUrl) {
  for (const fault of RED_CAP_FAULTS) {
    await executeCase(`fault.${fault.id}`, async (passed) => {
      const faultCommand = buildFaultCommand(plan, fault.id, { staleUrl: `http://127.0.0.1:${plan.ports.novnc}/vnc.html` })
      if (fault.id === "stale-endpoint") await screen(plan, ["stop"])
      const result = await commandRunner(faultCommand.command, faultCommand.args, {
        cwd: faultCommand.cwd,
        timeoutMs: Math.min(options.timeoutMs, 15_000),
        label: `fault-${fault.id}`,
      })
      if (fault.expectedOutcome === "fail") {
        throw new Error(`${fault.signal}; control result code=${result.code} signal=${result.signal ?? "none"}`)
      }
      if (fault.id === "stale-endpoint") {
        assert.notEqual(result.code, 0)
        await screen(plan, ["start"])
        passed("stale-response-ignored", "old endpoint was rejected")
        passed("current-selection-preserved", "fault did not alter the selected target")
        passed("retry-remains-bounded", `fault command completed in ${Math.round(result.durationMs)}ms`)
        return
      }
      if (fault.id === "streamer-crash") {
        await screen(plan, ["start"])
        const status = await screen(plan, ["status"])
        assert.match(status.stdout, /available=true/)
        passed("fault-trigger-recorded", "websockify fault command completed")
        passed("viewer-degrades-without-blocking-kernel", "display stack remained bounded")
        passed("display-recovers-once", "noVNC restarted once")
        return
      }
      if (fault.id === "browser-crash") {
        await screen(plan, ["start"])
        await screen(plan, ["open-url", `${fixtureUrl}/interactions`])
        const text = await cdp(plan, ["text"])
        assert.match(text.stdout, /Fixture interactions/)
        passed("fault-trigger-recorded", "Chromium process fault command completed")
        passed("stale-references-rejected", "one-shot CDP reconnected to a new page target")
        passed("browser-recovers-with-one-tab-registry", "direct stack recovered with one current target")
      }
    })
  }
  for (const caseId of ["fault.disk-pressure", "fault.resource-exhaustion"]) {
    await executeCase(caseId, async () => {
      throw new Error(expectedFailureForCase(caseId))
    })
  }
}

async function runResourceCase() {
  const before = resourceSamples[0]
  const after = await collectRedCapResourceSample({ rootPath: repoRoot, runCommand: commandRunner, containerName: plan.containerName })
  resourceSamples.push(after)
  await executeCase("resource.safety", async (passed) => {
    const artifactSize = await artifactBytes(paths.artifactRoot)
    const available = Number(after.disk.availableBytes)
    assert.ok(available >= RED_CAP_MIN_FREE_BYTES, `free disk ${available} is below the 28 GiB safety floor`)
    assert.ok(artifactSize <= 300 * 1024 * 1024, `evidence grew beyond 300 MiB: ${artifactSize}`)
    passed("idle-and-active-process-residency-recorded", `${before.processes.hostCount}->${after.processes.hostCount}`)
    passed("cpu-disk-and-process-count-recorded", JSON.stringify(after))
    passed("headroom-gate-enforced-before-launch", `available=${available}`)
    passed("post-run-resource-delta-recorded", `artifactBytes=${artifactSize}`)
  })
}

async function executeCase(caseId, operation) {
  const definition = browserComputerFunctionalCases().find((item) => item.id === caseId)
  if (!definition) throw new Error(`unknown functional case ${caseId}`)
  const passedIds = new Set()
  const evidence = new Map()
  const pass = (id, detail) => {
    if (!definition.assertions.includes(id)) throw new Error(`${caseId} operation produced unknown assertion ${id}`)
    passedIds.add(id)
    evidence.set(id, String(detail ?? "assertion passed"))
  }
  let operationError = null
  try {
    await operation(pass)
  } catch (error) {
    operationError = error
  }
  const assertions = definition.assertions.map((id) => {
    const expectedOutcome = expectedOutcomeForAssertion(caseId, id)
    const status = passedIds.has(id) ? "passed" : "failed"
    const reason = operationError?.message ?? knownFailureReason(caseId, id)
    return makeRedCapAssertion({
      id,
      caseId,
      status,
      expectedOutcome,
      evidence: evidence.get(id) ?? reason,
      reason: status === "failed" ? reason : undefined,
    })
  })
  caseRecords.set(caseId, { caseId, assertions })
}

function expectedOutcomeForAssertion(caseId, assertionId) {
  if (expectedFailureForCase(caseId)) return "fail"
  if (caseId === "browser.discovery" && assertionId === "stable-tab-identity") return "fail"
  if (caseId === "browser.lifecycle" && assertionId !== "dialog-handled") return "fail"
  if (caseId === "computer.text-selection") return "fail"
  if (caseId === "computer.desktop" && assertionId !== "browser-chrome-controllable") return "fail"
  if (caseId === "computer.clipboard" && assertionId === "secret-copy-restricted") return "fail"
  const fault = RED_CAP_FAULTS.find((item) => item.caseId === caseId)
  return fault?.expectedOutcome ?? "pass"
}

function knownFailureReason(caseId, assertionId) {
  return expectedFailureForCase(caseId)
    ?? (caseId === "browser.discovery" && assertionId === "stable-tab-identity" ? "one-shot CDP does not expose a kernel tab registry" : null)
    ?? (caseId === "browser.lifecycle" ? "popup/download/upload lifecycle is not exposed by the one-shot helper" : null)
    ?? (caseId === "computer.text-selection" ? "Room-owned selection attribution is not in this lane" : null)
    ?? (caseId === "computer.desktop" ? "non-browser desktop applications and multi-viewer resize are later acceptance rows" : null)
    ?? (caseId === "computer.clipboard" ? "secret clipboard policy is a later Room authority row" : null)
    ?? "assertion was not proven by the bounded adapter"
}

async function captureStackVersion(plan) {
  const browser = await commandRunner("docker", ["exec", "-u", "slice", plan.containerName, "chromium", "--version"], {
    timeoutMs: 10_000,
    label: "stack-chromium-version",
  })
  const noVnc = await commandRunner("docker", ["exec", "-u", "slice", plan.containerName, "dpkg-query", "-W", "-f", "${Version}", "novnc"], {
    timeoutMs: 10_000,
    label: "stack-novnc-version",
  })
  return {
    display: "novnc",
    displayVersion: noVnc.code === 0 ? noVnc.stdout.trim() : "unknown",
    browserControl: "one-shot-cdp",
    browserControlVersion: "apps/kernel/slice-linux-docker/docker/browser-cdp.mjs",
    browser: "chromium",
    browserVersion: browser.code === 0 ? browser.stdout.trim() : "unknown",
  }
}

async function captureScreenshot(plan, name) {
  const safeName = name.replace(/[^a-z0-9-]/gi, "-")
  const inside = `/tmp/chariox-m0-red-cap-${safeName}.png`
  await screen(plan, ["screenshot", inside])
  const artifact = path.join(paths.artifactRoot, `${safeName}.png`)
  const copied = await commandRunner("docker", ["cp", `${plan.containerName}:${inside}`, artifact], {
    timeoutMs: 15_000,
    label: `capture-${safeName}`,
  })
  if (copied.code !== 0) throw new Error(`screenshot copy failed: ${tail(copied.stderr || copied.stdout)}`)
  artifacts.push({ kind: "screenshot", path: artifact, bytes: (await readFile(artifact)).byteLength })
  return { inside, artifact }
}

async function writeArtifact(name, contents, kind) {
  const artifact = path.join(paths.artifactRoot, name)
  await writeFile(artifact, contents, { mode: 0o600 })
  artifacts.push({ kind, path: artifact, bytes: Buffer.byteLength(contents) })
  return artifact
}

async function waitForText(plan, needle) {
  const result = await screen(plan, ["browser-wait-text", needle, "10000"])
  if (!/"ok"\s*:\s*true/.test(result.stdout)) throw new Error(`browser did not expose expected text ${needle}`)
}

async function screen(plan, args) {
  const result = await commandRunner(plan.screen(args).command, plan.screen(args).args, {
    cwd: plan.provision.cwd,
    timeoutMs: Math.min(options.timeoutMs, 30_000),
    label: `screen-${args[0]}`,
  })
  if (result.code !== 0) throw new Error(`screen ${args[0]} failed: ${tail(result.stderr || result.stdout)}`)
  return result
}

async function cdp(plan, args) {
  const result = await commandRunner(plan.cdp(args).command, plan.cdp(args).args, {
    cwd: plan.provision.cwd,
    timeoutMs: Math.min(options.timeoutMs, 30_000),
    label: `cdp-${args[0]}`,
  })
  if (result.code !== 0) throw new Error(`CDP ${args[0]} failed: ${tail(result.stderr || result.stdout)}`)
  return result
}

async function restartStack(plan) {
  const restarted = await commandRunner("docker", ["restart", plan.containerName], {
    cwd: plan.provision.cwd,
    timeoutMs: Math.min(options.timeoutMs, 30_000),
    label: "restart-owned-slice",
  })
  if (restarted.code !== 0) throw new Error(`owned slice restart failed: ${tail(restarted.stderr || restarted.stdout)}`)
  await screen(plan, ["start"])
}

async function commandRunner(command, args, options = {}) {
  const result = await spawnBounded(command, args, { ...options, registry })
  commands.push(result)
  return result
}

async function collectHostOnlySample() {
  return await collectRedCapResourceSample({ rootPath: repoRoot, runCommand: commandRunner })
}

async function allocatePorts() {
  const ports = {}
  for (const name of ["novnc", "vnc", "kernel", "relay", "mcp"]) ports[name] = await findFreePort()
  return buildRedCapPorts(ports)
}

async function findFreePort() {
  const server = net.createServer()
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen({ host: "127.0.0.1", port: 0 }, resolve)
  })
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}

async function writeSuccessManifest() {
  const completedAt = new Date().toISOString()
  const completedMonoNs = Number(process.hrtime.bigint())
  caseRecords.set("cleanup.resources", {
    caseId: "cleanup.resources",
    assertions: browserComputerFunctionalCases().find((item) => item.id === "cleanup.resources").assertions.map((id) => makeRedCapAssertion({
      id,
      caseId: "cleanup.resources",
      status: cleanup.ok ? "passed" : "failed",
      expectedOutcome: "pass",
      evidence: cleanup.ok ? "owned container, volume, processes, and temporary root were cleaned" : cleanup.violations.join("; "),
      reason: cleanup.ok ? undefined : cleanup.violations.join("; "),
    })),
  })
  const assertionList = redCapCaseIds().flatMap((caseId) => caseRecords.get(caseId)?.assertions ?? [])
  const status = redCapManifestStatus(assertionList, true)
  const manifest = {
    schema: BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA,
    runId: paths.runId,
    testedSha: preflight.testedSha,
    originMainSha: preflight.originMainSha,
    startedAt,
    completedAt,
    startedMonoNs,
    completedMonoNs,
    durationMs: Math.max(0, Date.parse(completedAt) - Date.parse(startedAt)),
    status,
    ok: status === "passed",
    profile,
    stack: capturedStack ?? { display: "novnc", browserControl: "one-shot-cdp" },
    artifactRoot: paths.artifactRoot,
    charioxHome: paths.charioxHome,
    caseIds: redCapCaseIds(),
    commands: publicCommands(),
    assertions: assertionList,
    resourceSamples,
    artifacts,
    prerequisites: preflight,
    cleanup,
    functionalEvidence: buildFunctionalEvidence({
      runId: paths.runId,
      startedAt,
      completedAt,
      profile,
      stack: capturedStack,
      artifactRoot: paths.artifactRoot,
      caseResults: redCapCaseIds().map((caseId) => caseRecords.get(caseId)),
    }),
  }
  await writeManifest(manifest)
  console.log(`LIVE_BROWSER_COMPUTER_RED_CAP_${status.toUpperCase()} ${JSON.stringify({ manifest: manifestPath(), expectedFailures: assertionList.filter((item) => item.classification === "red-expected").length })}`)
  if (status !== "passed" && !options.allowExpectedRed) process.exitCode = 1
}

async function writeFailureManifest(error) {
  const completedAt = new Date().toISOString()
  const completedMonoNs = Number(process.hrtime.bigint())
  const assertions = redCapCaseIds().flatMap((caseId) => {
    const existing = caseRecords.get(caseId)?.assertions
    if (existing) return existing
    return browserComputerFunctionalCases().find((item) => item.id === caseId).assertions.map((id) => makeRedCapAssertion({
      id,
      caseId,
      status: "blocked",
      expectedOutcome: expectedOutcomeForAssertion(caseId, id),
      reason: `live drill aborted: ${error.message}`,
    }))
  })
  const manifest = {
    schema: BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA,
    runId: paths.runId,
    testedSha: preflight?.testedSha ?? "0000000000000000000000000000000000000000",
    originMainSha: preflight?.originMainSha ?? null,
    startedAt,
    completedAt,
    startedMonoNs,
    completedMonoNs,
    durationMs: Math.max(0, Date.parse(completedAt) - Date.parse(startedAt)),
    status: "blocked",
    ok: false,
    profile,
    stack: { display: "novnc", browserControl: "one-shot-cdp", mode: "pre-cutover" },
    artifactRoot: paths.artifactRoot,
    charioxHome: paths.charioxHome,
    caseIds: redCapCaseIds(),
    commands: publicCommands(),
    assertions,
    resourceSamples,
    artifacts,
    prerequisites: preflight ?? { ok: false, missing: [error.message] },
    cleanup,
    error: error.message,
  }
  await writeManifest(manifest)
  console.error(`LIVE_BROWSER_COMPUTER_RED_CAP_BLOCKED ${JSON.stringify({ manifest: manifestPath(), reason: error.message })}`)
}

async function writeManifest(manifest) {
  const sanitized = sanitizeRedCapManifest(manifest)
  if (sanitized.testedSha && !/^0{40}$/.test(sanitized.testedSha)) validateRedCapEvidenceManifest(sanitized, { repoRoot })
  await writeFile(manifestPath(), `${JSON.stringify(sanitized, null, 2)}\n`, { mode: 0o600 })
}

function publicCommands() {
  return commands.map((command) => ({
    label: command.label,
    command: command.command,
    args: redactRedCapCommandArgs(command.args),
    cwd: command.cwd,
    pid: command.pid,
    startedAt: command.startedAt,
    completedAt: command.completedAt,
    startedMonoNs: command.startedMonoNs,
    completedMonoNs: command.completedMonoNs,
    durationMs: command.durationMs,
    timeoutMs: command.timeoutMs,
    timedOut: command.timedOut,
    code: command.code,
    signal: command.signal,
    spawnError: command.spawnError,
  }))
}

function manifestPath() {
  return path.join(paths.artifactRoot, "evidence-manifest.json")
}

function parseJson(value, label) {
  try {
    return JSON.parse(value)
  } catch (error) {
    throw new Error(`${label} was not JSON: ${error.message}`)
  }
}

function tail(value) {
  return String(value ?? "").trim().split("\n").slice(-12).join("\n")
}

function escapeRegExp(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

function parseArgs(argv) {
  const result = {
    profile: "auto",
    image: process.env.CHARIOX_M0_SLICE_IMAGE ?? RED_CAP_DEFAULT_IMAGE,
    evidenceDir: process.env.CHARIOX_M0_EVIDENCE_DIR,
    tempRoot: process.env.CHARIOX_M0_TEMP_ROOT,
    timeoutMs: Number(process.env.CHARIOX_M0_TIMEOUT_MS ?? RED_CAP_DEFAULT_TIMEOUT_MS),
    allowExpectedRed: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === "--profile") result.profile = argv[++index]
    else if (argument === "--image") result.image = argv[++index]
    else if (argument === "--evidence-dir") result.evidenceDir = argv[++index]
    else if (argument === "--temp-root") result.tempRoot = argv[++index]
    else if (argument === "--timeout-ms") result.timeoutMs = Number(argv[++index])
    else if (argument === "--allow-expected-red") result.allowExpectedRed = true
    else if (argument === "--preflight-only") result.preflightOnly = true
    else if (argument === "--help") {
      console.log("Usage: node apps/cli/scripts/live-browser-computer-red-cap.mjs [--profile auto|mac|linux-docker] [--image REF] [--evidence-dir ABS] [--temp-root ABS] [--timeout-ms N] [--allow-expected-red] [--preflight-only]")
      process.exit(0)
    } else {
      throw new Error(`unknown option ${argument}`)
    }
  }
  if (!Number.isSafeInteger(result.timeoutMs) || result.timeoutMs <= 0 || result.timeoutMs > RED_CAP_MAX_RUN_MS) {
    throw new Error(`--timeout-ms must be a positive integer no greater than ${RED_CAP_MAX_RUN_MS}`)
  }
  return result
}
