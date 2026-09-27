#!/usr/bin/env node
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { readFile, realpath, stat, unlink, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const scriptPath = fileURLToPath(import.meta.url)
const repoRoot = path.resolve(path.dirname(scriptPath), "../../..")
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))
const maxHistoryActions = 4096
const pngSignature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])

function assertDrillDComputerAction(action, { actionId, agentId, baselineSequence }) {
  const valid = action?.action_id === actionId
    && Number.isSafeInteger(action.sequence) && action.sequence > baselineSequence
    && action.actor_id === `agent:${agentId}`
    && action.mode === "computer" && action.kind === "keyboard_text"
    && action.state === "completed" && action.outcome?.status === "completed"
    && Array.isArray(action.targets) && action.targets.some((target) => target?.kind === "desktop")
    && Number.isSafeInteger(action.arguments?.utf8_byte_count) && action.arguments.utf8_byte_count > 0
  if (!valid) {
    throw new Error("Drill D requires a completed Computer keyboard_text action targeting desktop, attributed to the expected agent after the Room baseline")
  }
  return action
}

function assertDrillDPreBrowserActions(actions, { baselineSequence, firstBrowserSequence, agentId }) {
  const actorId = `agent:${agentId}`
  const beforeBrowser = actions
    .filter((action) => action.sequence > baselineSequence && action.sequence < firstBrowserSequence)
    .sort((left, right) => left.sequence - right.sequence)
  assert.ok(beforeBrowser.length >= 2,
    "Drill D requires multiple attributed Computer actions before Browser work")
  for (const action of beforeBrowser) {
    assert.equal(action.actor_id, actorId, "pre-Browser action actor differs from the expected agent")
    assert.equal(action.mode, "computer", "Browser or non-Computer action occurred before explicit Browser work")
    assert.equal(action.state, "completed", "pre-Browser Computer action did not complete")
    assert.equal(action.outcome?.status, "completed", "pre-Browser Computer action has no completed outcome")
    assert.ok(action.targets?.some((target) => target?.kind === "desktop"), "pre-Browser Computer action did not target the desktop")
  }
  return beforeBrowser
}

function validateObservedComputerActions(actions, agentId) {
  const actorId = `agent:${agentId}`
  for (const action of actions) {
    assert.equal(action.actor_id, actorId, "pre-Browser action actor differs from the expected agent")
    assert.equal(action.mode, "computer", "Browser or non-Computer action occurred before explicit Browser work")
    assert.ok(action.targets?.some((target) => target?.kind === "desktop"),
      "pre-Browser Computer action did not target the desktop")
    assert.ok(["queued", "running", "completed"].includes(action.state),
      "pre-Browser Computer action failed or was cancelled")
    if (action.state === "completed") assert.equal(action.outcome?.status, "completed",
      "pre-Browser Computer action has no completed outcome")
  }
}

function browserStateProjection(environment, sessionId) {
  assert.equal(environment?.session_id, sessionId, "Room Environment belongs to a different session")
  assert.equal(environment.lifecycle, "ready", "Room Environment is not ready")
  assert.ok(typeof environment.environment_id === "string" && environment.environment_id.length > 0,
    "kernel omitted the Room Environment identity")
  assert.ok(Number.isSafeInteger(environment.runtime_generation) && environment.runtime_generation > 0,
    "kernel omitted the Room runtime generation")
  assert.ok(Array.isArray(environment.tabs) && environment.tabs.length > 0,
    "kernel omitted the Room browser tabs")
  assert.ok(typeof environment.focused_tab_id === "string" && environment.focused_tab_id.length > 0,
    "kernel omitted the focused Room browser tab")
  const tabs = environment.tabs.map((tab) => {
    assert.ok(typeof tab.tab_id === "string" && tab.tab_id.length > 0, "kernel omitted a browser tab identity")
    assert.ok(typeof tab.url === "string" && typeof tab.title === "string", "kernel omitted browser tab state")
    assert.ok(Number.isSafeInteger(tab.document_revision) && typeof tab.focused === "boolean",
      "kernel omitted browser tab revision or focus state")
    return {
      tabId: tab.tab_id,
      url: tab.url,
      title: tab.title,
      documentRevision: tab.document_revision,
      focused: tab.focused,
    }
  }).sort((left, right) => left.tabId.localeCompare(right.tabId))
  const focused = tabs.find((tab) => tab.tabId === environment.focused_tab_id)
  assert.ok(focused?.focused, "Room focused tab does not match the tab focus state")
  assert.equal(tabs.filter((tab) => tab.focused).length, 1, "Room browser has an ambiguous focused tab")
  const viewport = environment.viewport
  assert.ok(viewport && Number.isSafeInteger(viewport.revision)
    && Number.isSafeInteger(viewport.desktop_pixel_width) && viewport.desktop_pixel_width > 0
    && Number.isSafeInteger(viewport.desktop_pixel_height) && viewport.desktop_pixel_height > 0,
  "kernel omitted the Room desktop viewport")
  return {
    sessionId,
    environmentId: environment.environment_id,
    runtimeGeneration: environment.runtime_generation,
    focusedTabId: environment.focused_tab_id,
    tabs,
    viewport: {
      revision: viewport.revision,
      desktopPixelWidth: viewport.desktop_pixel_width,
      desktopPixelHeight: viewport.desktop_pixel_height,
    },
  }
}

export async function runDrillDAcceptance(options, dependencies = {}) {
  validateOptions(options)
  const LocalIpcClient = dependencies.LocalIpcClient
    ?? (await import("../../../packages/kernel-client/dist/ipc.js")).LocalIpcClient
  const requests = dependencies.requests
    ?? await import("../../../packages/kernel-client/dist/ipc-requests.js")
  const client = new LocalIpcClient(options.kernelUrl)
  const startedAt = new Date().toISOString()
  let attachmentId = null
  let failure = null
  let evidence = null
  let screenshots = null
  try {
    const attached = await client.send(requests.attachToSessionRequest(
      options.sessionId, `drill-d-acceptance-${process.pid}-${Date.now()}`,
    ))
    attachmentId = attached?.SessionAttached?.attachment?.id
    assert.ok(typeof attachmentId === "string" && attachmentId,
      "kernel omitted the attach-only Room observation identity")
    await waitForEditingCheckpoint(options)
    const baseline = await captureStableBaseline(client, requests, options.sessionId)
    baseline.screenshot = await captureRoomScreenshot(client, requests, options.sessionId, attachmentId,
      baseline.projection)
    await assertRoomUnchanged(client, requests, options.sessionId, baseline, "baseline screenshot capture")
    const desktopTransition = await waitForFirstBrowserAction(
      client, requests, options, baseline, attachmentId,
    )
    const finalOffice = await waitForFinalOfficeReport(options)
    const history = await readActionHistory(client, requests, options.sessionId)
    const finalEnvironment = await readEnvironment(client, requests, options.sessionId)
    const finalProjection = browserStateProjection(finalEnvironment, options.sessionId)
    assert.equal(finalProjection.environmentId, baseline.projection.environmentId,
      "Room Environment identity changed during Drill D")
    assert.equal(finalProjection.runtimeGeneration, baseline.projection.runtimeGeneration,
      "Room runtime generation changed during Drill D")
    const finalScreenshot = await captureRoomScreenshot(client, requests, options.sessionId, attachmentId,
      finalProjection)
    await assertRoomUnchanged(client, requests, options.sessionId, {
      ...baseline,
      projection: finalProjection,
      signature: actionSignature(history),
      sequence: maxSequence(history),
    }, "final browser screenshot capture")
    screenshots = {
      baseline: baseline.screenshot,
      editor: desktopTransition.editorScreenshot,
      final: finalScreenshot,
    }
    const proof = assertDrillDAcceptanceProof({
      options,
      office: finalOffice.office,
      phase: finalOffice.phase,
      history,
      baseline,
      preBrowserProjection: desktopTransition.preBrowserProjection,
      firstBrowser: desktopTransition.action,
      editorScreenshot: desktopTransition.editorScreenshot,
      baselineScreenshot: baseline.screenshot,
      finalScreenshot,
      finalProjection,
    })
    evidence = {
      schemaVersion: 1,
      drill: "browser-computer-drill-d",
      startedAt,
      completedAt: new Date().toISOString(),
      source: {
        kernelRequests: [
          "AttachToSession", "GetRoomEnvironmentState", "ListRoomEnvironmentActionHistory",
          "CaptureRoomEnvironmentScreenshot", "ReadRoomEnvironmentScreenshotChunk", "DetachFromSession",
        ],
        sessionId: options.sessionId,
        environmentId: baseline.projection.environmentId,
        runtimeGeneration: baseline.projection.runtimeGeneration,
        baselineSequence: baseline.sequence,
        baselineBrowserStateSha256: sha256Json(baseline.projection),
        baselineScreenshotSha256: baseline.screenshot.sha256,
        officeReportSha256: sha256Json(finalOffice),
        kernelActionHistorySha256: proof.actionHistorySha256,
        finalScreenshotSha256: finalScreenshot.sha256,
        preBrowserPollCount: baseline.polls,
      },
      editor: proof.editor,
      document: proof.document,
      browser: {
        stateBeforeFirstBrowserActionSha256: sha256Json(baseline.projection),
        stateAfterExplicitBrowserWorkSha256: sha256Json(finalProjection),
        firstActionId: desktopTransition.action.action_id,
        firstActionSequence: desktopTransition.action.sequence,
        firstActionKind: desktopTransition.action.kind,
        focusedTabIdAfterExplicitBrowserWork: finalProjection.focusedTabId,
        activationActionId: proof.browser.activationActionId,
        uploadActionId: proof.browser.uploadActionId,
        submitActionId: proof.browser.submitActionId,
      },
      screenshots: screenshotEvidence(options.evidenceOut, baseline.screenshot,
        desktopTransition.editorScreenshot, finalScreenshot),
      kernelActionIds: proof.kernelActionIds,
    }
  } catch (error) {
    failure = error
    throw error
  } finally {
    let finalizerError = null
    if (attachmentId) {
      try {
        await client.send(requests.detachFromSessionRequest(attachmentId))
      } catch (error) {
        if (!failure) finalizerError = new Error(`failed to detach the read-only Room observer: ${error.message}`)
      }
    }
    try {
      await client.close?.()
    } catch (error) {
      if (!failure && !finalizerError) finalizerError = new Error(`failed to close the Room observer connection: ${error.message}`)
    }
    if (finalizerError) throw finalizerError
  }
  await writePrivateEvidence(options.evidenceOut, evidence, screenshots)
  return evidence
}

async function captureStableBaseline(client, requests, sessionId) {
  const historyBefore = await readActionHistory(client, requests, sessionId)
  const environmentBefore = await readEnvironment(client, requests, sessionId)
  const historyAfter = await readActionHistory(client, requests, sessionId)
  const environmentAfter = await readEnvironment(client, requests, sessionId)
  const projectionBefore = browserStateProjection(environmentBefore, sessionId)
  const projectionAfter = browserStateProjection(environmentAfter, sessionId)
  assert.deepEqual(projectionAfter, projectionBefore, "Room browser state changed while the acceptance baseline was captured")
  assert.equal(maxSequence(historyAfter), maxSequence(historyBefore),
    "Room action sequence advanced while the acceptance baseline was captured; start the checker before the desktop task")
  assert.deepEqual(actionSignature(historyAfter), actionSignature(historyBefore),
    "Room action history changed while the acceptance baseline was captured; start the checker before the desktop task")
  assert.ok(!historyAfter.some((action) => action.state === "queued" || action.state === "running"),
    "Room has an unfinished action at the acceptance baseline")
  return {
    sequence: maxSequence(historyAfter),
    signature: actionSignature(historyAfter),
    projection: projectionAfter,
    polls: 0,
  }
}

async function assertRoomUnchanged(client, requests, sessionId, baseline, phase) {
  const environment = await readEnvironment(client, requests, sessionId)
  assert.deepEqual(browserStateProjection(environment, sessionId), baseline.projection,
    `Room browser state changed during ${phase}`)
  const history = await readActionHistory(client, requests, sessionId)
  assert.equal(maxSequence(history), baseline.sequence, `Room action history advanced during ${phase}`)
  assert.deepEqual(actionSignature(history), baseline.signature, `Room action history changed during ${phase}`)
}

async function waitForFirstBrowserAction(client, requests, options, baseline, attachmentId) {
  const deadline = Date.now() + options.timeoutMs
  let polls = 0
  let editorScreenshot = null
  let preBrowserProjection = null
  while (Date.now() < deadline) {
    polls += 1
    const historyBeforeState = await readActionHistory(client, requests, options.sessionId)
    const existing = firstPostBaselineBrowser(historyBeforeState, baseline.sequence)
    if (existing) {
      assert.ok(editorScreenshot, "Browser work began before the editor UI evidence was captured")
      assert.equal(existing.actor_id, `agent:${options.agentId}`, "first Browser action is not attributed to the expected agent")
      baseline.polls = polls
      return { action: existing, editorScreenshot, preBrowserProjection }
    }

    const environment = await readEnvironment(client, requests, options.sessionId)
    const projection = browserStateProjection(environment, options.sessionId)
    assert.equal(projection.environmentId, baseline.projection.environmentId, "Room Environment identity changed before Browser work")
    assert.equal(projection.runtimeGeneration, baseline.projection.runtimeGeneration, "Room runtime generation changed before Browser work")
    assert.deepEqual(projection, baseline.projection,
      "Room browser focus or tab state changed before an explicit Browser action")
    preBrowserProjection = projection

    const historyAfterState = await readActionHistory(client, requests, options.sessionId)
    const appeared = firstPostBaselineBrowser(historyAfterState, baseline.sequence)
    if (appeared) {
      assert.ok(editorScreenshot, "Browser work began before the editor UI evidence was captured")
      assert.deepEqual(projection, baseline.projection,
        "Room browser state changed in the interval before its first Browser action could be ordered")
      assert.equal(appeared.actor_id, `agent:${options.agentId}`, "first Browser action is not attributed to the expected agent")
      baseline.polls = polls
      return { action: appeared, editorScreenshot, preBrowserProjection: projection }
    }
    const observedComputerActions = historyAfterState.filter((action) => action.sequence > baseline.sequence)
    if (observedComputerActions.length) {
      validateObservedComputerActions(observedComputerActions, options.agentId)
    }
    if (!editorScreenshot) {
      const reportPath = await canonicalOutsideRepoPath(options.officeReport, "office report")
      const checkpoint = await readCheckpoint(reportPath).catch((error) => {
        if (error?.code === "ENOENT") return null
        throw error
      })
      if (checkpoint?.phase === "office-failed" || checkpoint?.phase === "failed") {
        throw new Error("live office drill failed before the editor screenshot")
      }
      if (["office-passed", "passed"].includes(checkpoint?.phase)) {
        throw new Error("live office drill finished without a pre-Browser editor screenshot")
      }
      if (checkpoint?.phase === "office-mailing") {
        const office = checkpoint.office
        assert.equal(office?.agentId, options.agentId, "office editor checkpoint belongs to a different agent")
        assert.equal(office?.edit?.exactDocument, true, "live editor checkpoint did not verify the saved document")
        assert.equal(office?.edit?.focusPreserved, true, "live editor checkpoint did not verify editor focus")
        const typed = assertDrillDComputerAction(
          historyAfterState.find((action) => action.action_id === office.edit.typedActionId),
          { actionId: office.edit.typedActionId, agentId: options.agentId, baselineSequence: baseline.sequence },
        )
        assert.equal(typed.sequence, office.edit.typedSequence, "editor checkpoint action sequence differs from kernel history")
        assertDrillDPreBrowserActions(historyAfterState, {
          baselineSequence: baseline.sequence,
          firstBrowserSequence: Number.MAX_SAFE_INTEGER,
          agentId: options.agentId,
        })
        editorScreenshot = await captureRoomScreenshot(
          client, requests, options.sessionId, attachmentId, baseline.projection,
        )
        const environmentAfterCapture = await readEnvironment(client, requests, options.sessionId)
        assert.deepEqual(browserStateProjection(environmentAfterCapture, options.sessionId), baseline.projection,
          "Room browser state changed during the pre-Browser editor screenshot")
        const historyAfterCapture = await readActionHistory(client, requests, options.sessionId)
        assert.deepEqual(actionSignature(historyAfterCapture), actionSignature(historyAfterState),
          "Room action history changed during the pre-Browser editor screenshot")
        assert.equal(firstPostBaselineBrowser(historyAfterCapture, baseline.sequence), null,
          "Browser work overlapped the pre-Browser editor screenshot")
      }
    }
    await sleep(options.pollMs)
  }
  throw new Error("timed out before an explicit Browser action followed the desktop Computer work")
}

async function waitForFinalOfficeReport(options) {
  const deadline = Date.now() + options.timeoutMs
  while (Date.now() < deadline) {
    try {
      const reportPath = await canonicalOutsideRepoPath(options.officeReport, "office report")
      const report = await readCheckpoint(reportPath)
      if (!report) {
        await sleep(options.pollMs)
        continue
      }
      if (report?.phase === "failed" || report?.phase === "office-failed") {
        throw new Error("live office drill reported a failed phase")
      }
      if (report?.phase === "passed" && report.office?.mail?.received) return report
    } catch (error) {
      if (error?.code !== "ENOENT") throw error
    }
    await sleep(options.pollMs)
  }
  throw new Error("timed out waiting for the completed live office report with saved-document and Browser evidence")
}

async function waitForEditingCheckpoint(options) {
  const deadline = Date.now() + options.timeoutMs
  while (Date.now() < deadline) {
    try {
      const reportPath = await canonicalOutsideRepoPath(options.officeReport, "office report")
      const report = await readCheckpoint(reportPath)
      if (report?.phase === "office-failed" || report?.phase === "failed") {
        throw new Error("live office drill failed before the desktop editor task")
      }
      if (["office-mailing", "office-passed", "passed"].includes(report?.phase)) {
        throw new Error("office editor task already started; launch this watcher before its first Computer action")
      }
      if (report?.phase === "office-editing") {
        assert.equal(report.office?.agentId, options.agentId,
          "office editing checkpoint belongs to a different agent")
        assert.ok(typeof report.office?.document === "string" && path.posix.isAbsolute(report.office.document),
          "office editing checkpoint omitted its saved-document identity")
        return
      }
    } catch (error) {
      if (error?.code !== "ENOENT") throw error
    }
    await sleep(options.pollMs)
  }
  throw new Error("timed out waiting for the live office-editing checkpoint after Browser fixture setup")
}

async function readCheckpoint(reportPath) {
  const reportStat = await stat(reportPath)
  assert.ok(reportStat.isFile() && reportStat.size <= 2 * 1024 * 1024,
    "live office report is not a bounded regular file")
  try {
    return JSON.parse(await readFile(reportPath, "utf8"))
  } catch (error) {
    if (error instanceof SyntaxError) return null
    throw error
  }
}

export function assertDrillDAcceptanceProof({
  options, office, phase, history, baseline, firstBrowser, baselineScreenshot,
  preBrowserProjection, editorScreenshot, finalScreenshot, finalProjection,
}) {
  const actorId = `agent:${options.agentId}`
  assert.equal(phase, "passed", "live office drill did not finish successfully")
  assert.equal(baseline.projection.sessionId, options.sessionId, "baseline Room identity differs from the requested session")
  assert.equal(preBrowserProjection?.sessionId, options.sessionId,
    "pre-Browser Room identity differs from the requested session")
  assert.deepEqual(preBrowserProjection, baseline.projection,
    "Room browser state changed before the first explicit Browser action")
  assert.equal(finalProjection.sessionId, options.sessionId, "final Room identity differs from the requested session")
  assert.equal(finalProjection.environmentId, baseline.projection.environmentId,
    "Room Environment identity changed during Drill D")
  assert.equal(finalProjection.runtimeGeneration, baseline.projection.runtimeGeneration,
    "Room runtime generation changed during Drill D")
  assert.equal(office?.agentId, options.agentId, "live office report belongs to a different agent")
  assert.equal(office?.fixtureClosed, true, "live office fixture cleanup was not reported")
  assert.equal(office?.edit?.exactDocument, true, "live office UI did not verify the saved document")
  assert.equal(office?.edit?.focusPreserved, true, "live office UI did not verify editor focus preservation")
  assert.ok(typeof office?.edit?.typedActionId === "string" && office.edit.typedActionId,
    "live office report omitted its attributed Computer action id")
  assert.ok(Number.isSafeInteger(office.edit.typedSequence), "live office report omitted its Computer action sequence")
  assert.match(office?.installed ?? "", /(?:^|\n)mousepad\s+\S+/i,
    "live office report omitted the installed graphical editor")
  assert.match(office?.installed ?? "", /(?:^|\n)xterm\s+\S+/i,
    "live office report omitted the installed desktop terminal")
  assert.equal(office?.desktop?.editorDefaultSettings, true, "editor did not use the installed default settings backend")
  assert.equal(office?.desktop?.editorSessionBus, true, "editor did not use the shared desktop session bus")
  assert.equal(office?.mail?.visibleBrowser, true, "live office report did not observe the Browser after its explicit action")
  assert.equal(office?.mail?.submissions, 1, "live office report did not observe one completed attachment submission")
  assert.ok(typeof office?.document === "string" && path.posix.isAbsolute(office.document),
    "live office report omitted the saved document identity")

  const received = office.mail.received
  assert.ok(typeof received.name === "string" && received.name === path.posix.basename(office.document),
    "saved document name differs from the observed attachment")
  assert.ok(Number.isSafeInteger(received.sizeBytes) && received.sizeBytes > 0,
    "saved document has no actual attachment byte count")
  assert.match(received.sha256 ?? "", /^[a-f0-9]{64}$/, "saved document has no actual attachment SHA-256")

  assertScreenshotMatchesRoom(baselineScreenshot, baseline.projection, "initial Browser")
  assertScreenshotMatchesRoom(editorScreenshot, baseline.projection, "saved editor")
  assertScreenshotMatchesRoom(finalScreenshot, finalProjection, "final Browser")
  const actionById = new Map(history.map((action) => [action.action_id, action]))
  const typedAction = actionById.get(office.edit.typedActionId)
  assert.ok(typedAction, "kernel history omitted the reported Computer edit action")
  const typed = assertDrillDComputerAction(typedAction, {
    actionId: office.edit.typedActionId,
    agentId: options.agentId,
    baselineSequence: baseline.sequence,
  })
  assert.equal(typed.sequence, office.edit.typedSequence, "reported Computer action sequence differs from kernel history")
  assert.equal(typed.arguments.utf8_byte_count, received.sizeBytes,
    "saved document byte count differs from the attributed graphical text action")

  const allPostBaseline = history.filter((action) => action.sequence > baseline.sequence)
  const firstBrowserInHistory = firstPostBaselineBrowser(history, baseline.sequence)
  assert.ok(firstBrowserInHistory, "kernel history omitted the first explicit Browser action")
  assert.equal(firstBrowserInHistory.action_id, firstBrowser.action_id, "observed first Browser action changed in final history")
  assert.equal(firstBrowserInHistory.sequence, firstBrowser.sequence, "observed first Browser action sequence changed")
  assert.ok(typed.sequence < firstBrowser.sequence, "Browser work began before the reported desktop edit")
  const beforeBrowser = assertDrillDPreBrowserActions(allPostBaseline, {
    baselineSequence: baseline.sequence,
    firstBrowserSequence: firstBrowser.sequence,
    agentId: options.agentId,
  })
  assert.ok(beforeBrowser.some((action) => action.action_id === typed.action_id),
    "attributed document edit was not part of the Computer action sequence")

  for (const action of allPostBaseline.filter((item) => item.sequence >= firstBrowser.sequence)) {
    assert.equal(action.actor_id, actorId, "post-desktop action actor differs from the expected agent")
    assert.equal(action.mode, "browser", "Computer work resumed after Browser work began")
    assert.equal(action.state, "completed", "Browser action did not complete")
    assert.equal(action.outcome?.status, "completed", "Browser action has no completed outcome")
  }
  assert.equal(firstBrowser.actor_id, actorId)
  assert.equal(firstBrowser.mode, "browser")
  assert.equal(firstBrowser.state, "completed")

  const activation = requireAction(actionById, office.mail.activationActionId, actorId, "browser", "browser_tab_activate")
  const upload = requireAction(actionById, office.mail.uploadActionId, actorId, "browser", "upload")
  const submit = requireAction(actionById, office.mail.submitActionId, actorId, "browser", "submit")
  assert.deepEqual(
    allPostBaseline.filter((action) => action.mode === "browser" && action.kind === "submit")
      .map((action) => action.action_id),
    [submit.action_id],
    "kernel action history contains an unexpected duplicate or missing attachment submission",
  )
  assert.ok(firstBrowser.sequence <= activation.sequence && activation.sequence < upload.sequence && upload.sequence < submit.sequence,
    "explicit Browser actions are absent or out of order")
  const uploadTab = requireBrowserTabTarget(upload)
  assert.deepEqual(activation.targets, [{ kind: "desktop" }, uploadTab],
    "Browser activation did not explicitly focus the same Room tab used for upload")
  assert.deepEqual(submit.targets, [uploadTab], "attachment submission targeted a different Room tab")
  assert.equal(finalProjection.focusedTabId, uploadTab.id,
    "final Room browser focus differs from the explicit upload tab")
  assert.ok(finalProjection.tabs.some((tab) => tab.tabId === uploadTab.id && tab.focused),
    "final kernel snapshot omitted the focused Browser tab")

  return {
    editor: {
      application: "Mousepad",
      installedPackages: ["mousepad", "xterm"],
      activeWindowPreserved: true,
      screenshotSha256: editorScreenshot.sha256,
      screenshotWidth: editorScreenshot.width,
      screenshotHeight: editorScreenshot.height,
      screenshotArtifactId: editorScreenshot.artifactId,
      screenshotMatchesRoomViewport: true,
      typedActionId: typed.action_id,
      typedActionSequence: typed.sequence,
      typedActionActorId: typed.actor_id,
      typedActionTarget: "desktop",
    },
    document: {
      name: received.name,
      sizeBytes: received.sizeBytes,
      sha256: received.sha256,
      savedFileVerifiedByLiveOfficeDriver: true,
    },
    browser: {
      activationActionId: activation.action_id,
      uploadActionId: upload.action_id,
      submitActionId: submit.action_id,
      focusedTabId: uploadTab.id,
      baselineScreenshotSha256: baselineScreenshot.sha256,
      finalScreenshotSha256: finalScreenshot.sha256,
      finalScreenshotArtifactId: finalScreenshot.artifactId,
    },
    actionHistorySha256: sha256Json(allPostBaseline.slice().sort((left, right) => left.sequence - right.sequence)
      .map((action) => ({
        actionId: action.action_id,
        sequence: action.sequence,
        actorId: action.actor_id,
        mode: action.mode,
        kind: action.kind,
        targets: action.targets,
        state: action.state,
        outcome: action.outcome,
      }))),
    kernelActionIds: allPostBaseline.slice().sort((left, right) => left.sequence - right.sequence)
      .map((action) => action.action_id),
  }
}

function requireAction(actionById, actionId, actorId, mode, kind) {
  assert.ok(typeof actionId === "string" && actionId, `live office report omitted the ${kind} action id`)
  const action = actionById.get(actionId)
  assert.ok(action, `kernel history omitted the reported ${kind} action`)
  assert.equal(action.actor_id, actorId, `${kind} action actor differs`)
  assert.equal(action.mode, mode, `${kind} action mode differs`)
  assert.equal(action.kind, kind, `${kind} action kind differs`)
  assert.equal(action.state, "completed", `${kind} action did not complete`)
  assert.equal(action.outcome?.status, "completed", `${kind} action has no completed outcome`)
  return action
}

function requireBrowserTabTarget(action) {
  assert.equal(action.targets?.length, 1, "Browser upload must target one Room tab")
  const target = action.targets[0]
  assert.ok(target?.kind === "browser_tab" && typeof target.id === "string" && target.id,
    "Browser upload omitted its Room tab target")
  return { kind: "browser_tab", id: target.id }
}

async function captureRoomScreenshot(client, requests, sessionId, attachmentId, projection) {
  const response = await client.send(requests.captureRoomEnvironmentScreenshotRequest(sessionId, attachmentId))
  const artifact = response?.RoomEnvironmentScreenshotCaptured?.artifact
  assert.ok(artifact && typeof artifact.artifact_id === "string" && artifact.artifact_id,
    "kernel omitted the Room screenshot artifact identity")
  assert.equal(artifact.media_type, "image/png", "kernel screenshot is not PNG evidence")
  assert.ok(Number.isSafeInteger(artifact.size_bytes) && artifact.size_bytes > pngSignature.length
    && artifact.size_bytes <= 16 * 1024 * 1024, "kernel screenshot exceeds the bounded evidence size")
  assert.match(artifact.sha256 ?? "", /^[a-f0-9]{64}$/, "kernel screenshot omitted its SHA-256")
  const chunks = []
  let offset = 0
  while (offset < artifact.size_bytes) {
    const chunkResponse = await client.send(requests.readRoomEnvironmentScreenshotChunkRequest(
      sessionId, attachmentId, artifact.artifact_id, offset, 128 * 1024,
    ))
    const chunk = chunkResponse?.RoomEnvironmentScreenshotChunk?.chunk
    assert.ok(chunk && chunk.artifact_id === artifact.artifact_id && chunk.offset === offset,
      "kernel screenshot chunks are missing, reordered, or cross-scoped")
    assert.ok(typeof chunk.data_base64 === "string" && chunk.data_base64.length > 0
      && /^[A-Za-z0-9+/]+={0,2}$/.test(chunk.data_base64), "kernel screenshot chunk has invalid Base64")
    const bytes = Buffer.from(chunk.data_base64, "base64")
    assert.ok(bytes.length > 0 && bytes.length <= 128 * 1024 && offset + bytes.length <= artifact.size_bytes,
      "kernel screenshot chunk has an invalid size")
    chunks.push(bytes)
    offset += bytes.length
    assert.equal(chunk.eof, offset === artifact.size_bytes,
      "kernel screenshot EOF does not match its declared size")
  }
  const bytes = Buffer.concat(chunks)
  assert.equal(bytes.length, artifact.size_bytes, "kernel screenshot size differs from its declaration")
  assert.equal(sha256(bytes), artifact.sha256, "kernel screenshot SHA-256 verification failed")
  assert.ok(bytes.subarray(0, pngSignature.length).equals(pngSignature), "kernel screenshot has no PNG signature")
  assert.equal(bytes.toString("ascii", 12, 16), "IHDR", "kernel screenshot omitted the PNG image header")
  assert.ok(bytes.length >= 24, "kernel screenshot is too short to contain PNG dimensions")
  const width = bytes.readUInt32BE(16)
  const height = bytes.readUInt32BE(20)
  const capture = { artifactId: artifact.artifact_id, sha256: artifact.sha256, sizeBytes: bytes.length, width, height, bytes }
  assertScreenshotMatchesRoom(capture, projection, "captured Room")
  return capture
}

function assertScreenshotMatchesRoom(screenshot, projection, label) {
  assert.ok(screenshot?.bytes && screenshot.bytes.length === screenshot.sizeBytes,
    `${label} screenshot bytes are missing`)
  assert.equal(screenshot.width, projection.viewport.desktopPixelWidth,
    `${label} screenshot width differs from the Room viewport`)
  assert.equal(screenshot.height, projection.viewport.desktopPixelHeight,
    `${label} screenshot height differs from the Room viewport`)
  assert.equal(sha256(screenshot.bytes), screenshot.sha256, `${label} screenshot bytes changed after capture`)
}

function screenshotEvidencePaths(evidenceOut) {
  const parsed = path.parse(evidenceOut)
  return {
    beforeEditor: path.join(parsed.dir, `${parsed.name}.before-editor.png`),
    editor: path.join(parsed.dir, `${parsed.name}.editor.png`),
    afterBrowser: path.join(parsed.dir, `${parsed.name}.after-browser.png`),
  }
}

function screenshotEvidence(evidenceOut, baseline, editor, final) {
  const paths = screenshotEvidencePaths(evidenceOut)
  const describe = (screenshot, outputPath) => ({
    path: outputPath,
    artifactId: screenshot.artifactId,
    sha256: screenshot.sha256,
    sizeBytes: screenshot.sizeBytes,
    width: screenshot.width,
    height: screenshot.height,
  })
  return {
    initialBrowser: describe(baseline, paths.beforeEditor),
    editor: describe(editor, paths.editor),
    finalBrowser: describe(final, paths.afterBrowser),
  }
}

async function readEnvironment(client, requests, sessionId) {
  const response = await client.send(requests.getRoomEnvironmentStateRequest(sessionId))
  const environment = response?.RoomEnvironmentState?.environment
  assert.ok(environment, "kernel omitted the public Room Environment state")
  return environment
}

async function readActionHistory(client, requests, sessionId) {
  const actions = []
  const ids = new Set()
  const sequences = new Set()
  let beforeSequence = null
  while (true) {
    const response = await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, beforeSequence, 100))
    const page = response?.RoomEnvironmentActionHistoryListed?.page
    assert.ok(Array.isArray(page?.actions) && page.actions.length <= 100,
      "kernel omitted a valid Room action-history page")
    for (const action of page.actions) {
      assert.ok(Number.isSafeInteger(action.sequence) && action.sequence > 0,
        "kernel action history omitted a valid sequence")
      assert.ok(typeof action.action_id === "string" && action.action_id && !ids.has(action.action_id),
        "kernel action history has a missing or duplicate action identity")
      assert.ok(!sequences.has(action.sequence), "kernel action history has a duplicate sequence")
      assert.ok(beforeSequence === null || action.sequence < beforeSequence,
        "kernel action-history cursor did not advance")
      ids.add(action.action_id)
      sequences.add(action.sequence)
      actions.push(action)
      assert.ok(actions.length <= maxHistoryActions, "Room action history exceeds the acceptance evidence bound")
    }
    const next = page.next_before_sequence
    if (next == null) return actions
    assert.ok(page.actions.length > 0 && Number.isSafeInteger(next) && next > 0
      && (beforeSequence === null || next < beforeSequence), "kernel action-history cursor did not advance")
    beforeSequence = next
  }
}

function firstPostBaselineBrowser(actions, baselineSequence) {
  return actions.filter((action) => action.sequence > baselineSequence && action.mode === "browser")
    .sort((left, right) => left.sequence - right.sequence)[0] ?? null
}

function maxSequence(actions) {
  return Math.max(0, ...actions.map((action) => action.sequence))
}

function actionSignature(actions) {
  return actions.slice().sort((left, right) => left.sequence - right.sequence).map((action) => ({
    actionId: action.action_id,
    sequence: action.sequence,
    state: action.state,
    outcome: action.outcome,
  }))
}

function validateOptions(options) {
  for (const name of ["kernelUrl", "sessionId", "agentId", "officeReport", "evidenceOut"]) {
    assert.ok(typeof options?.[name] === "string" && options[name].trim(), `--${kebab(name)} is required`)
  }
  const url = new URL(options.kernelUrl)
  assert.ok(["ws:", "wss:"].includes(url.protocol) && !url.username && !url.password
    && (url.pathname === "/" || url.pathname === "") && !url.search && !url.hash,
  "--kernel-url must be a credential-free ws:// or wss:// endpoint without a token path")
  for (const name of ["officeReport", "evidenceOut"]) {
    assert.ok(path.isAbsolute(options[name]), `--${kebab(name)} must be an absolute path`)
  }
  assert.equal(path.extname(options.evidenceOut), ".json", "--evidence-out must name a JSON file")
  assert.notEqual(path.resolve(options.officeReport), path.resolve(options.evidenceOut),
    "--office-report and --evidence-out must be different files")
  assert.ok(Number.isSafeInteger(options.pollMs) && options.pollMs >= 50 && options.pollMs <= 5_000,
    "--poll-ms must be between 50 and 5000")
  assert.ok(Number.isSafeInteger(options.timeoutMs) && options.timeoutMs >= 1_000 && options.timeoutMs <= 3_600_000,
    "--timeout-ms must be between 1000 and 3600000")
}

function parseArguments(argv) {
  const options = { kernelUrl: null, sessionId: null, agentId: null, officeReport: null, evidenceOut: null,
    pollMs: 100, timeoutMs: 1_200_000 }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    const value = () => {
      const next = argv[index + 1]
      if (!next || next.startsWith("--")) throw new Error(`missing value for ${arg}`)
      index += 1
      return next
    }
    if (arg === "--kernel-url") options.kernelUrl = value()
    else if (arg === "--session-id") options.sessionId = value()
    else if (arg === "--agent-id") options.agentId = value()
    else if (arg === "--office-report") options.officeReport = path.resolve(value())
    else if (arg === "--evidence-out") options.evidenceOut = path.resolve(value())
    else if (arg === "--poll-ms") options.pollMs = parseInteger(value(), arg)
    else if (arg === "--timeout-ms") options.timeoutMs = parseInteger(value(), arg)
    else if (arg === "--help" || arg === "-h") return { help: true }
    else throw new Error(`unknown option: ${arg}`)
  }
  validateOptions(options)
  return options
}

async function canonicalOutsideRepoPath(inputPath, label) {
  assert.ok(path.isAbsolute(inputPath), `${label} path must be absolute`)
  const parent = await realpath(path.dirname(inputPath))
  const candidate = path.join(parent, path.basename(inputPath))
  let canonical = candidate
  try {
    canonical = await realpath(candidate)
  } catch (error) {
    if (error?.code !== "ENOENT") throw error
  }
  assert.ok(!isWithin(repoRoot, canonical), `${label} must be outside the repository`)
  return canonical
}

async function writePrivateEvidence(outputPath, evidence, screenshots) {
  const canonical = await canonicalOutsideRepoPath(outputPath, "evidence output")
  const directory = path.dirname(canonical)
  const directoryStat = await stat(directory)
  assert.ok(directoryStat.isDirectory(), "evidence output parent is not a directory")
  const screenshotPaths = screenshotEvidencePaths(canonical)
  const artifacts = [
    [screenshotPaths.beforeEditor, screenshots.baseline],
    [screenshotPaths.editor, screenshots.editor],
    [screenshotPaths.afterBrowser, screenshots.final],
  ]
  const written = []
  try {
    for (const [output, screenshot] of artifacts) {
      assert.equal(sha256(screenshot.bytes), screenshot.sha256, "screenshot evidence changed before publication")
      const target = await canonicalOutsideRepoPath(output, "screenshot evidence output")
      await writeFile(target, screenshot.bytes, { flag: "wx", mode: 0o600 })
      written.push(target)
    }
    await writeFile(canonical, `${JSON.stringify(evidence, null, 2)}\n`, { flag: "wx", mode: 0o600 })
    written.push(canonical)
  } catch (error) {
    await Promise.all(written.map((file) => unlink(file).catch(() => {})))
    throw error
  }
}

function isWithin(root, candidate) {
  const relative = path.relative(root, candidate)
  return relative === "" || (relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex")
}

function sha256Json(value) {
  return sha256(Buffer.from(JSON.stringify(value), "utf8"))
}

function parseInteger(value, option) {
  const number = Number(value)
  if (!Number.isSafeInteger(number)) throw new Error(`${option} must be an integer`)
  return number
}

function kebab(value) {
  return value.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)
}

function printUsage() {
  console.log(`Usage: node apps/cli/scripts/live-browser-computer-drill-d.mjs \\
  --kernel-url ws://127.0.0.1:PORT --session-id ID --agent-id ID \\
  --office-report /absolute/private/office-report.json \\
  --evidence-out /absolute/private/drill-d-evidence.json [--poll-ms 100] [--timeout-ms 1200000]

Start this read-only watcher before the office fixture begins. It waits for the fixture's office-editing checkpoint, captures a public Room baseline, then waits for the completed report. It sends no Room actions.`)
}

async function main() {
  const options = parseArguments(process.argv.slice(2))
  if (options.help) {
    printUsage()
    return
  }
  const evidence = await runDrillDAcceptance(options)
  console.log(`Drill D acceptance passed: ${evidence.document.sha256} (${evidence.document.sizeBytes} bytes)`)
  console.log(`Evidence: ${options.evidenceOut}`)
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main().catch((error) => {
    console.error(`Drill D acceptance failed: ${error.message}`)
    process.exitCode = 1
  })
}
