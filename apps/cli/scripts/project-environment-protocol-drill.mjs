#!/usr/bin/env node
// MP-03/MP-08/MP-10: live protocol 487, using product-authorized client profiles.
// Build kernel-client first. Run against a disposable kernel with no live sessions.
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../../packages/kernel-client/dist/ipc.js"
import * as requests from "../../../packages/kernel-client/dist/ipc-requests.js"

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..")
const options = {}
for (let i = 2; i < process.argv.length; i += 2) {
  const name = process.argv[i]
  assert(["--kernel-url", "--foreign-client-file", "--evidence-root", "--workspace"].includes(name), "unknown drill argument")
  assert(process.argv[i + 1], "missing drill argument")
  options[name] = process.argv[i + 1]
}
assert(options["--kernel-url"] && options["--foreign-client-file"] && options["--evidence-root"],
  "requires --kernel-url, --foreign-client-file (a product-authorized different user), and --evidence-root")
const evidence = path.resolve(options["--evidence-root"])
assert(evidence !== repo && !evidence.startsWith(repo + path.sep), "evidence must be outside the checkout")
assert(process.env.CHARIOX_HOME && path.isAbsolute(process.env.CHARIOX_HOME), "explicit disposable CHARIOX_HOME required")
await mkdir(evidence, { recursive: true })
const privateProfile = path.resolve(options["--foreign-client-file"])
assert.equal((await stat(privateProfile)).mode & 0o077, 0, "foreign client profile must be private")
// Never include the profile, authorization, identity or diagnostic error payload in evidence.
const profile = JSON.parse(await readFile(privateProfile, "utf8"))
assert(profile.endpoint && profile.options, "foreign profile requires endpoint and normal LocalIpcClient options")
const owner = new LocalIpcClient(options["--kernel-url"])
const foreign = new LocalIpcClient(profile.endpoint, profile.options)
const report = { mp: ["MP-03", "MP-08", "MP-10"], protocol: 487, scope: "live owned Project read without a live session/agent (ended bootstrap history retained); authenticated foreign-owner denial; reserved Export without side effects", checks: [], cleanup: null }
let projectId, sessionId
let stage = "empty-kernel-precondition"
async function liveSessions() {
  return (await owner.send(requests.listSessionsRequest())).SessionsListed.sessions.filter(session => session.status !== "Ended")
}
async function directoryDigest() {
  const home = process.env.CHARIOX_HOME
  const kernels = await readdir(path.join(home, "kernels"), { withFileTypes: true }).catch(error => { if (error.code === "ENOENT") return []; throw error })
  const directories = [path.join(home, "state", "project-environments"), ...kernels.filter(entry => entry.isDirectory()).map(kernel => path.join(home, "kernels", kernel.name, "project-environments"))]
  const hash = createHash("sha256")
  let found = false
  for (const directory of directories.sort()) {
    const names = await readdir(directory).catch(error => { if (error.code === "ENOENT") return []; throw error })
    if (names.length) found = true
    for (const name of names.sort()) {
      hash.update(directory); hash.update(name); hash.update(await readFile(path.join(directory, name)))
    }
  }
  assert(found, "must hash the real kernel environment store")
  return hash.digest("hex")
}
try {
  assert.equal((await liveSessions()).length, 0, "use an empty disposable kernel")
  const workspace = path.resolve(options["--workspace"] ?? repo)
  const created = (await owner.send(requests.createSessionRequest(workspace, workspace, "Protocol 487", undefined, null, null, null, null, { kind: "new" }))).SessionCreated
  assert(created)
  projectId = created.session.project_id; sessionId = created.session.id
  const bootstrap = (await owner.send(requests.getSessionStateRequest(sessionId))).SessionState.session
  for (const agent of bootstrap.agents) await owner.send(requests.destroyAgentRequest(sessionId, agent.id))
  await owner.send(requests.endSessionRequest(sessionId))
  // Deleting the final Session also deletes its Project. End preserves normal
  // history while retiring every live Session and agent, without a fixture.
  const ended = (await owner.send(requests.getSessionStateRequest(sessionId))).SessionState.session
  assert.equal(ended.status, "Ended"); assert.equal(ended.agents.length, 0)
  report.bootstrapSessionEnded = true
  report.endedBootstrapHistoryRetained = true
  assert.equal((await liveSessions()).length, 0)
  stage = "owned-get-no-session-or-agent"
  const snapshot = (await owner.send(requests.getProjectEnvironmentRequest(projectId))).ProjectEnvironment.environment
  assert.equal(snapshot.local_project_id, projectId)
  assert.deepEqual(snapshot.delivered_capabilities.enabled_environment_operations, ["get"])
  assert.deepEqual(snapshot.observations, []); assert.deepEqual(snapshot.operations, [])
  report.checks.push({ name: "owned-get-no-session-or-agent", passed: true })
  stage = "authenticated-foreign-owner-denial"
  let denied = false
  try { await foreign.send(requests.getProjectEnvironmentRequest(projectId)) }
  catch (error) { denied = /does not own/.test(String(error.message)) }
  assert(denied, "foreign client must reach the Project owner check, not merely fail relay admission")
  report.checks.push({ name: "authenticated-foreign-owner-denial", passed: true })
  stage = "reserved-export-no-side-effects"
  const before = await directoryDigest()
  const destination = path.join(evidence, "must-not-be-exported.json")
  const response = await owner.send({ ExportProjectEnvironment: {
    projectId, operationId: "protocol-487-reserved-export", expectedRevision: snapshot.revision,
    revisionDigest: snapshot.content_digest, selectedItems: [], selectedFiles: [], destination: { kind: "file", path: destination },
  } })
  assert.deepEqual(response, { EnvironmentUnsupportedFeature: { feature: "export", supported_schema: 1 } })
  assert.equal(await directoryDigest(), before)
  await assert.rejects(stat(destination), { code: "ENOENT" })
  assert.deepEqual((await owner.send(requests.getProjectEnvironmentRequest(projectId))).ProjectEnvironment.environment, snapshot)
  assert.equal((await liveSessions()).length, 0)
  report.checks.push({ name: "reserved-export-no-side-effects", passed: true })
  report.result = "PASS"
} catch (error) {
  // The error code alone is payload-free and names the failing mechanism.
  report.result = "FAIL"; report.firstFailingSeam = stage; report.firstFailureCode = error?.code ?? error?.name; process.exitCode = 1
} finally {
  try {
    if (projectId) {
      const remaining = (await owner.send(requests.listProjectsRequest(true))).ProjectsListed.projects
      if (remaining.some(project => project.id === projectId)) await owner.send(requests.deleteProjectRequest(projectId))
    }
    report.cleanup = { sessions: (await liveSessions()).length, ownedProjectRemoved: !(await owner.send(requests.listProjectsRequest(true))).ProjectsListed.projects.some(project => project.id === projectId) }
    assert.equal(report.cleanup.sessions, 0); assert(report.cleanup.ownedProjectRemoved)
  } catch { report.result = "FAIL"; report.cleanup = { failed: true }; process.exitCode = 1 }
  owner.close(); foreign.close()
  await writeFile(path.join(evidence, "protocol-487.json"), JSON.stringify(report, null, 2) + "\n")
}
console.log(JSON.stringify(report))
