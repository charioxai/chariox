// MP-08 / MP-10 / MP-11: the terminal Skills surface drives the home catalog
// through the same requests as the web Skills screen.
import assert from "node:assert/strict"
import test from "node:test"
import { createCommandActionHandlers } from "./command-actions.js"
import { makeAgent, makeCommandDeps, makeSession } from "./command-actions-test-support.js"
import type { NativeSkillEntry, SkillProjection, SkillsV2Request, SkillsV2Response } from "@chariox/kernel-client/kernel-types-skills-v2"

const projection = (id: string, name: string, overrides: Partial<SkillProjection> = {}): SkillProjection => ({
  skill: {
    id, name, scope: { kind: "chariox" }, package_id: null, description: `${name} instructions`, short_description: null,
    default_state: "auto", state_override: null, digest: `${id}`.padEnd(64, "0"), declared_version: null, files: [], compatibility_notes: [],
    provenance: { kind: "folder", source_machine: null, locator: `/skills/${name}`, source_revision: null, provider: null, account_profile: null, installed_at_ms: 0, updated_at_ms: 0 },
  },
  effective_state: "auto", project_override: null, delivery: { kind: "chariox" }, also_in_provider: null, ...overrides,
})

const native: NativeSkillEntry = {
  selection: { source: "native", entry_id: "claude:lint", name: "lint", provider: "claude", account_profile: "default", execution_id: "run-1", inventory_revision: "inv-1" },
  description: "Lint with house rules", digest: null, exposure: "confirmed", default_state: "auto", complete_manifest: true, supports_load_ack: true, discovery_error: null,
}

function harness(packages: Extract<SkillsV2Response, { kind: "catalog" }>["packages"] = [], failure?: Error,
  repositoryUpdate?: Extract<SkillsV2Response, { kind: "repository_update" }>, agentProvider = "claude") {
  const requests: SkillsV2Request[] = []
  const notices: string[] = []
  const footers: string[] = []
  const catalog: SkillsV2Response = {
    kind: "catalog", revision: 7, packages, native: [native], metadata_budget_percent: 1,
    skills: [
      projection("skill-review", "review", { also_in_provider: "claude" }),
      projection("skill-deploy", "deploy", { delivery: { kind: "conflict", provider: "claude", entry_id: "claude:deploy", native_digest: "f".repeat(64) } }),
    ],
  }
  const session = makeSession({ project_id: "project-1", focused_agent_id: "agent-1", agents: [makeAgent({ id: "agent-1", provider: agentProvider })] })
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => session,
    focusedAgentId: () => "agent-1",
    appendNotice: (message: string) => notices.push(message),
    flashFooter: (message: string) => footers.push(message),
    sendSkillsRequest: async ({ SkillsV2: request }: { SkillsV2: SkillsV2Request }): Promise<SkillsV2Response> => {
      requests.push(request)
      if (failure) throw failure
      switch (request.action) {
        case "catalog": return catalog
        case "loads": return { kind: "loads", records: [] }
        case "preview": return { kind: "preview", expected_revision: request.mutation.expected_revision, affected: [], collisions: [], review_receipt: { id: "receipt-1", expires_at_ms: 0, mutation_sha256: "x", sources: [] } }
        case "resolve_repository": return { kind: "repository", locator: request.url, revision: "c".repeat(40),
          skills: [{ path: "skills/alpha", name: "alpha", description: "A" }, { path: "skills/beta", name: "beta", description: "B" }],
          skipped: [{ path: "skills/bad", reason: "SKILL.md must start with YAML frontmatter" }] }
        case "resolve_repository_update": {
          if (!repositoryUpdate) throw new Error("unexpected repository update")
          return repositoryUpdate
        }
        case "mutate": return { kind: "published", revision: 8, changed_skill_ids: ["skill-review"], removed_skill_ids: [] }
        default: throw new Error(`unexpected ${request.action}`)
      }
    },
  }))
  const run = (raw: string) => handlers.handleSkillCommand({ kind: "skill", raw, args: raw.replace(/^\/skills?\s*/, "").split(/\s+/).filter(Boolean) })
  return { requests, notices, footers, run }
}

test("/skills list shows the home catalog with states, also-in and conflict rows", async () => {
  const { requests, notices, run } = harness()
  await run("/skills list")
  assert.deepEqual(requests[0], { action: "catalog", project_id: "project-1", agent_id: "agent-1" })
  const text = notices.join("\n")
  assert.match(text, /\$review {2}Auto · Inherited · also in Claude/)
  assert.match(text, /\$deploy {2}Auto · Inherited · conflicting bytes in Claude; Chariox wins \$deploy/)
})

test("/skills list names a source-backed package's revision", async () => {
  const { notices, run } = harness([{ id: "pkg-1", name: "pocock", scope: { kind: "chariox" }, custom: false, source_locator: "https://github.com/mattpocock/skills", source_revision: "d1caf1e", upstream_members: {} }])
  await run("/skills list")
  assert.match(notices.join("\n"), /package pocock \(https:\/\/github.com\/mattpocock\/skills @ d1caf1e\) · 0 skill\(s\) \[pkg-1\]/)
})

test("a refused /skills command keeps the reason in the transcript without the transport wrappers", async () => {
  const refusal = "kernel transport `handle kernel response` failed: local transport `skills.v2.catalog` failed: skill name `review` is occupied; choose Rename or Replace"
  const { notices, footers, run } = harness([], new Error(refusal))
  await run("/skills list")
  assert.deepEqual(notices, ["Skills: skill name `review` is occupied; choose Rename or Replace"])
  assert.deepEqual(footers, ["Skills: skill name `review` is occupied; choose Rename or Replace"])
})

test("/skills tells what to do next when the kernel does not answer in time", async () => {
  const { notices, footers, run } = harness([], new Error("kernel transport `handle kernel response` failed: timed out"))
  await run("/skills repo-update Acme")
  const expected = "Skills: the kernel did not answer in time and may still be working. Run /skills list to see whether the change landed, then repeat the command if it did not."
  assert.deepEqual(notices, [expected])
  assert.deepEqual(footers, [expected])
})

test("/skills from lists the focused provider's skills read-only", async () => {
  const { notices, run } = harness()
  await run("/skills from")
  assert.match(notices.join("\n"), /\$lint {2}From Claude · confirmed · Auto/)
})

for (const launchMode of ["claude-headless", "claude-p"]) {
  test(`/skills from, show-from and import find the canonical Claude skills of a ${launchMode} agent`, async () => {
    const { notices, footers, requests, run } = harness([], undefined, undefined, launchMode)
    await run("/skills from")
    assert.match(notices.join("\n"), /\$lint {2}From Claude · confirmed · Auto/)
    await run("/skills show-from lint")
    await run("/skills import lint")
    assert.ok(requests.some(request => request.action === "preview"))
    assert.deepEqual(footers.filter(footer => /lists no skill/.test(footer)), [])
  })
}

test("/skills state previews a Project override and confirm publishes the reviewed batch", async () => {
  const { requests, notices, run } = harness()
  await run("/skills state review off --project")
  const preview = requests.find(request => request.action === "preview")
  assert.deepEqual(preview, { action: "preview", mutation: { scope: { kind: "project", project_id: "project-1" }, expected_revision: 7,
    actions: [{ action: "set_state", skill_id: "skill-review", state: "off" }] } })
  await run("/skills confirm")
  const mutate = requests.find(request => request.action === "mutate")
  assert.deepEqual(mutate, { action: "mutate", review_receipt_id: "receipt-1", mutation: preview!.action === "preview" ? preview!.mutation : null })
  assert.match(notices.at(-1)!, /Published Skills revision 8/)
})

test("/skills state reset clears the Chariox override; a second confirm has nothing to publish", async () => {
  const { requests, footers, run } = harness()
  await run("/skills state $deploy reset")
  const preview = requests.find(request => request.action === "preview")
  assert.deepEqual(preview?.action === "preview" && preview.mutation.actions, [{ action: "set_state", skill_id: "skill-deploy", state: null }])
  assert.deepEqual(preview?.action === "preview" && preview.mutation.scope, { kind: "chariox" })
  await run("/skills cancel")
  await run("/skills confirm")
  assert.equal(requests.filter(request => request.action === "mutate").length, 0)
  assert.match(footers.at(-1)!, /nothing to confirm/)
})

test("/skills import reviews installing the focused provider's skill", async () => {
  const { requests, run } = harness()
  await run("/skills import lint --rename house-lint")
  const preview = requests.find(request => request.action === "preview")
  assert.deepEqual(preview?.action === "preview" && preview.mutation.actions, [{ action: "install", source: { kind: "native", selection: native.selection },
    collision: { action: "rename", name: "house-lint" }, package_id: null }])
})

test("/skills repo reviews one source-backed package of every skill at the resolved commit", async () => {
  const { requests, notices, run } = harness()
  await run("/skills repo https://github.com/acme/skills.git")
  assert.deepEqual(requests.find(request => request.action === "resolve_repository"), { action: "resolve_repository", url: "https://github.com/acme/skills.git" })
  const preview = requests.find(request => request.action === "preview")
  const actions = preview?.action === "preview" ? preview.mutation.actions : []
  const created = actions[0]
  assert.equal(created?.action === "create_package" && created.name, "skills")
  const packageId = created?.action === "create_package" ? created.package_id : ""
  assert.deepEqual(actions.slice(1), ["skills/alpha", "skills/beta"].map(path => ({
    action: "install", source: { kind: "repository", url: "https://github.com/acme/skills.git", revision: "c".repeat(40), path },
    collision: { action: "reject" }, package_id: packageId,
  })))
  assert.match(notices.join("\n"), /skipped skills\/bad: SKILL.md must start with YAML frontmatter/)
  assert.match(notices.join("\n"), /Review Skills changes .*: create_package skills, install ×2 from https:\/\/github.com\/acme\/skills.git @ c{40}/)
})

test("/skills repo --only installs just the named directories and refuses an unknown one", async () => {
  const { requests, notices, footers, run } = harness()
  await run("/skills repo https://github.com/acme/skills.git --only skills/beta --name Acme")
  const preview = requests.find(request => request.action === "preview")
  const actions = preview?.action === "preview" ? preview.mutation.actions : []
  assert.equal(actions.length, 2)
  assert.equal(actions[0]?.action === "create_package" && actions[0].name, "Acme")
  assert.equal(actions[1]?.action === "install" && actions[1].source.kind === "repository" && actions[1].source.path, "skills/beta")
  await run("/skills repo https://github.com/acme/skills.git --only skills/nope")
  assert.match(footers.at(-1)!, /no skill at skills\/nope/)
  assert.match(notices.at(-2)!, /\$alpha {2}skills\/alpha/, "the listing is shown before the refusal")
})


test("/skills repo-update reviews the resolved commit before publishing the kernel batch", async () => {
  const update: Extract<SkillsV2Response, { kind: "repository_update" }> = {
    kind: "repository_update", package_id: "package-repo", locator: "https://github.com/acme/skills.git",
    from_revision: "a".repeat(40), revision: "b".repeat(40), added: [], removed: [], unchanged: [],
    changed: [{ member: { path: "skills/review", skill_id: "skill-review", name: "review", digest: "d".repeat(64) }, digest: "e".repeat(64) }],
    skipped: [{ path: "skills/moved", reason: "moved out of this package; retained" }],
    mutation: { scope: { kind: "chariox" }, expected_revision: 9, actions: [{ action: "update", skill_id: "skill-review", expected_digest: "d".repeat(64), source: { kind: "repository", url: "https://github.com/acme/skills.git", revision: "b".repeat(40), path: "skills/review" } }] },
  }
  const { requests, notices, run } = harness([{ id: "package-repo", name: "Acme", scope: { kind: "chariox" }, custom: false,
    source_locator: update.locator, source_revision: update.from_revision, upstream_members: { "skills/review": "skill-review" } }], undefined, update)
  await run("/skills repo-update Acme")
  assert.deepEqual(requests.find(request => request.action === "resolve_repository_update"), { action: "resolve_repository_update", package_id: "package-repo" })
  const preview = requests.find(request => request.action === "preview")
  assert.deepEqual(preview?.action === "preview" && preview.mutation, update.mutation)
  assert.equal(requests.some(request => request.action === "mutate"), false)
  assert.match(notices.join("\n"), /changed \$review/)
  assert.match(notices.join("\n"), /skipped skills\/moved: moved out/)
  await run("/skills confirm")
  const mutation = requests.find(request => request.action === "mutate")
  assert.deepEqual(mutation?.action === "mutate" && mutation.mutation, update.mutation)
})

test("/skills update-package does not stage an up-to-date repository", async () => {
  const revision = "a".repeat(40)
  const { requests, footers, run } = harness([{ id: "package-repo", name: "Acme", scope: { kind: "chariox" }, custom: false,
    source_locator: "https://github.com/acme/skills.git", source_revision: revision, upstream_members: {} }], undefined,
    { kind: "repository_update", package_id: "package-repo", locator: "https://github.com/acme/skills.git", from_revision: revision, revision,
      changed: [], added: [], removed: [], unchanged: [], skipped: [], mutation: { scope: { kind: "chariox" }, expected_revision: 7, actions: [] } })
  await run("/skills update-package Acme")
  assert.match(footers.at(-1)!, /up to date/)
  assert.equal(requests.some(request => request.action === "preview"), false)
  await run("/skills confirm")
  assert.equal(requests.some(request => request.action === "mutate"), false)
})
