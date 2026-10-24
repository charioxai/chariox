// MP-08 / MP-10 / MP-11: the terminal Skills surface. It sends the same home
// catalog requests as the web Skills screen (catalog, get, native details,
// preview, mutate); the kernel resolves states, scopes and conflicts.
import { randomUUID } from "node:crypto"
import {
  mutateSkillCatalogRequest,
  nativeSkillDetailsRequest,
  previewSkillCatalogRequest,
  skillCatalogRequest,
  skillsV2Request,
} from "@chariox/kernel-client/ipc-skills-v2-requests"
import type {
  NativeSkillEntry,
  SkillCatalogAction,
  SkillCatalogMutation,
  SkillInstallSource,
  SkillPackageRecord,
  SkillProjection,
  SkillReviewReceipt,
  SkillScope,
  SkillState,
  SkillsV2Request,
  SkillsV2Response,
} from "@chariox/kernel-client/kernel-types-skills-v2"

type Catalog = Extract<SkillsV2Response, { kind: "catalog" }>
type Preview = Extract<SkillsV2Response, { kind: "preview" }>

export type SkillsCommandContext = {
  sessionId: string | null
  projectId: string | null
  agentId: string | null
  provider: string | null
}

export type SkillsCommandDeps = {
  send: (request: { SkillsV2: SkillsV2Request }) => Promise<SkillsV2Response>
  context: () => SkillsCommandContext
  appendNotice: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

const USAGE = "usage: /skills list | chariox | project | environment | from | show <skill> | show-from <name> | "
  + "install <folder|url> [--project] [--revision <rev>] [--rename <name> | --replace <skill>] [--package <package>] | "
  + "import <name> [--project] [--rename <name> | --replace <skill>] | repo <https-url> [--project] [--name <package>] [--only <path,path>] | update <skill> [<folder|url>] | remove <skill> | "
  + "state <skill> auto|manual|off|reset [--project] | create-package <name> [--project] | move-to-package <skill> <package> | "
  + "ungroup <skill> | package-state <package> auto|manual|off|reset [--project] | update-package <package> | repo-update <package> | "
  + "remove-package <package> | ungroup-package <package> | confirm | cancel"

// A reply that never came says nothing about the kernel's work: it may still be running.
const UNANSWERED = "the kernel did not answer in time and may still be working. Run /skills list to see whether the change landed, then repeat the command if it did not."
const PROVIDER_LABELS: Record<string, string> = { codex: "Codex", claude: "Claude", opencode: "OpenCode" }
const providerLabel = (provider: string): string => PROVIDER_LABELS[provider] ?? provider
const stateLabel = (state: SkillState): string => state[0]!.toUpperCase() + state.slice(1)

class SkillsCommandError extends Error {}

type Flags = { positional: string[]; project: boolean; values: Map<string, string> }

function parseFlags(args: readonly string[]): Flags {
  const flags: Flags = { positional: [], project: false, values: new Map() }
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!
    if (arg === "--project") flags.project = true
    else if (arg === "--revision" || arg === "--rename" || arg === "--replace" || arg === "--package" || arg === "--name" || arg === "--only") {
      const value = args[index + 1]
      if (!value) throw new SkillsCommandError(`${arg} needs a value`)
      flags.values.set(arg.slice(2), value)
      index += 1
    } else flags.positional.push(arg)
  }
  return flags
}

function parseState(value: string | undefined): SkillState | null {
  if (value === "auto" || value === "manual" || value === "off") return value
  if (value === "reset") return null
  throw new SkillsCommandError("state must be auto, manual, off or reset")
}

function sourceFrom(value: string, revision: string | null): SkillInstallSource {
  if (/^https?:\/\//.test(value)) return { kind: "url", url: value, pinned_revision: revision }
  if (!value.startsWith("/")) throw new SkillsCommandError("give an absolute folder on the kernel machine or an https URL")
  return { kind: "folder", path: value }
}

function recordedSource(row: SkillProjection): SkillInstallSource | null {
  const locator = row.skill.provenance.locator
  if (!locator || row.skill.provenance.kind === "repository") return null
  if (row.skill.provenance.kind === "folder") return { kind: "folder", path: locator }
  return /^https?:\/\//.test(locator) ? { kind: "url", url: locator, pinned_revision: null } : null
}

function deliveryNote(row: SkillProjection): string {
  if (row.also_in_provider) {
    const off = row.effective_state === "off" ? "; the provider copy stays available" : ""
    return `also in ${providerLabel(row.also_in_provider)}${off}`
  }
  switch (row.delivery.kind) {
    case "conflict": return `conflicting bytes in ${providerLabel(row.delivery.provider)}; Chariox wins $${row.skill.name}`
    case "unverified": return `${providerLabel(row.delivery.provider)} inventory unverified`
    case "provided_by": return `provided by ${providerLabel(row.delivery.provider)}`
    case "chariox": return "Chariox delivery"
  }
}

function overrideNote(row: SkillProjection): string {
  if (row.project_override) return "Project override"
  return row.skill.state_override ? "Skill override" : "Inherited"
}

export function createSkillsCommand(deps: SkillsCommandDeps): (args: readonly string[]) => Promise<void> {
  let review: { mutation: SkillCatalogMutation; preview: Preview } | null = null

  const request = async <K extends SkillsV2Response["kind"]>(
    body: { SkillsV2: SkillsV2Request },
    kind: K,
  ): Promise<Extract<SkillsV2Response, { kind: K }>> => {
    const result = await deps.send(body)
    if (result.kind !== kind) throw new SkillsCommandError(`the kernel answered ${result.kind} instead of ${kind}; refresh with /skills list`)
    return result as Extract<SkillsV2Response, { kind: K }>
  }

  const catalog = (): Promise<Catalog> => {
    const context = deps.context()
    return request(skillCatalogRequest(context.projectId, context.agentId), "catalog")
  }

  const loadedIds = async (): Promise<Map<string, string>> => {
    const context = deps.context()
    if (!context.sessionId) return new Map()
    const loads = await request(skillsV2Request({ action: "loads", session_id: context.sessionId, agent_id: context.agentId }), "loads")
    return new Map(loads.records.map(record => [record.skill_id, record.digest]))
  }

  const requireProject = (): SkillScope => {
    const projectId = deps.context().projectId
    if (!projectId) throw new SkillsCommandError("this session has no Project: choose or create one in the Waiting Room to manage Project skills")
    return { kind: "project", project_id: projectId }
  }

  const findSkill = (result: Catalog, ref: string | undefined): SkillProjection => {
    if (!ref) throw new SkillsCommandError("name the skill (name or id)")
    const name = ref.replace(/^\$/, "")
    const byId = result.skills.find(row => row.skill.id === ref)
    if (byId) return byId
    const matches = result.skills.filter(row => row.skill.name === name)
    if (matches.length === 1) return matches[0]!
    if (matches.length > 1) throw new SkillsCommandError(`$${name} exists in more than one scope: use its id (${matches.map(row => row.skill.id).join(", ")})`)
    throw new SkillsCommandError(`no installed skill named ${ref}; see /skills list`)
  }

  const findPackage = (result: Catalog, ref: string | undefined): SkillPackageRecord => {
    if (!ref) throw new SkillsCommandError("name the package (name or id)")
    const matches = result.packages.filter(record => record.id === ref || record.name === ref)
    if (matches.length === 1) return matches[0]!
    if (matches.length > 1) throw new SkillsCommandError(`more than one package is named ${ref}: use its id`)
    throw new SkillsCommandError(`no package named ${ref}; see /skills list`)
  }

  const nativeEntries = (result: Catalog): NativeSkillEntry[] => {
    const provider = deps.context().provider
    return result.native.filter(entry => entry.selection.source === "native" && entry.selection.provider === provider)
  }

  const findNative = (result: Catalog, name: string | undefined): NativeSkillEntry => {
    const context = deps.context()
    if (!context.agentId || !context.provider) throw new SkillsCommandError("focus a provider agent to use its skills")
    if (!name) throw new SkillsCommandError(`name the ${providerLabel(context.provider)} skill`)
    const entry = nativeEntries(result).find(candidate => candidate.selection.name === name.replace(/^\$/, ""))
    if (!entry) throw new SkillsCommandError(`${providerLabel(context.provider)} lists no skill named ${name}; see /skills from`)
    return entry
  }

  const scopeSection = (result: Catalog, scope: SkillScope["kind"], loads: Map<string, string>): string[] => {
    const rows = result.skills.filter(row => row.skill.scope.kind === scope)
    const packages = result.packages.filter(record => record.scope.kind === scope)
    const lines: string[] = []
    const render = (row: SkillProjection) => {
      const loaded = loads.get(row.skill.id)
      const loadNote = loaded === undefined ? "" : loaded === row.skill.digest ? " · loaded in this session" : " · earlier revision loaded"
      lines.push(`  $${row.skill.name}  ${stateLabel(row.effective_state)} · ${overrideNote(row)} · ${deliveryNote(row)}${loadNote}`)
      lines.push(`      ${row.skill.short_description || row.skill.description} [${row.skill.declared_version || row.skill.digest.slice(0, 12)} · ${row.skill.id}]`)
    }
    for (const record of packages) {
      const members = rows.filter(row => row.skill.package_id === record.id)
      lines.push(`  package ${record.name} (${record.custom ? "custom" : record.source_locator ? `${record.source_locator}${record.source_revision ? ` @ ${record.source_revision}` : ""}` : "no source"}) · ${members.length} skill(s) [${record.id}]`)
      members.forEach(render)
    }
    rows.filter(row => !row.skill.package_id || !packages.some(record => record.id === row.skill.package_id)).forEach(render)
    if (!rows.length && !packages.length) lines.push("  no skills installed")
    return lines
  }

  const showCatalog = async (scopes: readonly SkillScope["kind"][]) => {
    const [result, loads] = await Promise.all([catalog(), loadedIds()])
    const lines = [`Skills · catalog revision ${result.revision} · metadata budget ${result.metadata_budget_percent}% · changes apply next turn`]
    for (const scope of scopes) {
      if (scope === "project" && !deps.context().projectId) {
        lines.push("Project: this session has no Project (choose or create one in the Waiting Room)")
        continue
      }
      lines.push(scope === "chariox" ? "Chariox:" : "Project:")
      lines.push(...scopeSection(result, scope, loads))
    }
    if (scopes.includes("project") && !scopes.includes("chariox") && deps.context().projectId) {
      lines.push("Chariox dependencies (Project overrides with /skills state <skill> <state> --project):")
      for (const row of result.skills.filter(candidate => candidate.skill.scope.kind === "chariox")) {
        lines.push(`  $${row.skill.name}  ${stateLabel(row.effective_state)} · ${row.project_override ? "Project override" : "Inherited"}`)
      }
    }
    deps.appendNotice(lines.join("\n"))
    deps.flashFooter(`Skills revision ${result.revision}`, "info")
  }

  const showFrom = async () => {
    const context = deps.context()
    if (!context.agentId || !context.provider) throw new SkillsCommandError("focus a provider agent to see skills From its provider")
    const label = providerLabel(context.provider)
    const entries = nativeEntries(await catalog())
    const lines = [`From ${label} (read-only; /skills import <name> creates a separate Chariox revision):`]
    for (const entry of entries) {
      lines.push(`  $${entry.selection.name}  From ${label} · ${entry.exposure} · ${stateLabel(entry.default_state)} · ${entry.digest ? entry.digest.slice(0, 12) : "digest unverified"}`)
      lines.push(`      ${entry.description}${entry.discovery_error ? ` (${entry.discovery_error})` : ""}`)
    }
    if (!entries.length) lines.push(`  No ${label} skills are loaded yet. The provider reads its skills when it starts: send this agent a message, then run /skills from again.`)
    deps.appendNotice(lines.join("\n"))
    deps.flashFooter(`${entries.length} skill(s) From ${label}`, "info")
  }

  const show = async (ref: string | undefined) => {
    const row = findSkill(await catalog(), ref)
    const detail = await request(skillsV2Request({ action: "get", skill_id: row.skill.id, project_id: deps.context().projectId }), "detail")
    const skill = detail.skill.skill
    deps.appendNotice([
      `$${skill.name} · ${skill.scope.kind === "chariox" ? "Chariox" : "Project"} · ${stateLabel(detail.skill.effective_state)} · ${deliveryNote(detail.skill)}`,
      `digest ${skill.digest} · ${skill.declared_version || "no declared version"} · ${skill.provenance.locator || skill.provenance.kind} ${skill.provenance.source_revision ?? ""}`.trimEnd(),
      ...skill.files.map(file => `  ${file.path} · ${file.bytes} bytes · ${file.executable ? "executable" : "file"} · ${file.sha256}`),
      ...skill.compatibility_notes.map(note => `  note: ${note}`),
      "",
      detail.body,
    ].join("\n"))
  }

  const showFromDetails = async (name: string | undefined) => {
    const entry = findNative(await catalog(), name)
    if (entry.selection.source !== "native") throw new SkillsCommandError("not a provider skill")
    const { details } = await request(nativeSkillDetailsRequest(deps.context().agentId!, entry.selection), "native_details")
    const label = providerLabel(entry.selection.provider)
    deps.appendNotice([
      `$${entry.selection.name} · From ${label} (read-only) · ${details.entry.exposure} · ${details.declared_version || "no declared version"}`,
      `digest ${details.entry.digest || "unverified"} · ${details.entry.complete_manifest ? "complete file manifest" : "file manifest incomplete"}`,
      `${details.provenance.locator || "source location unavailable"} · ${details.provenance.source_machine || "source machine unknown"}`,
      ...(details.unavailable_reason ? [details.unavailable_reason] : []),
      ...details.files.map(file => `  ${file.path} · ${file.bytes} bytes · ${file.executable ? "executable" : "file"} · ${file.sha256}`),
      ...details.compatibility_notes.map(note => `  note: ${note}`),
      "",
      details.body ?? "Body unavailable; the provider source has not been copied or imported.",
    ].join("\n"))
  }

  const stage = async (result: Catalog, scope: SkillScope, actions: SkillCatalogAction[]) => {
    const mutation: SkillCatalogMutation = { scope, expected_revision: result.revision, actions }
    deps.flashFooter("Reviewing with the home kernel", "info")
    const preview = await request(previewSkillCatalogRequest(mutation), "preview")
    review = { mutation, preview }
    deps.appendNotice(formatReview(mutation, preview))
    deps.flashFooter("Review ready: /skills confirm publishes, /skills cancel discards", "info")
  }

  const collision = (result: Catalog, flags: Flags): Extract<SkillCatalogAction, { action: "install" }>["collision"] => {
    const rename = flags.values.get("rename")
    const replace = flags.values.get("replace")
    if (rename && replace) throw new SkillsCommandError("use either --rename or --replace")
    if (rename) return { action: "rename", name: rename }
    if (replace) return { action: "replace", skill_id: findSkill(result, replace).skill.id }
    return { action: "reject" }
  }

  const skillScope = (row: SkillProjection, project: boolean): SkillScope =>
    project || row.skill.scope.kind === "project" ? requireProject() : { kind: "chariox" }

  const confirm = async () => {
    const current = review
    if (!current) throw new SkillsCommandError("nothing to confirm: stage a change first")
    review = null
    const receipt: SkillReviewReceipt = current.preview.review_receipt
    const published = await request(mutateSkillCatalogRequest(current.mutation, receipt.id), "published")
    deps.appendNotice(`Published Skills revision ${published.revision}: ${published.changed_skill_ids.length} changed, ${published.removed_skill_ids.length} removed. Agents use it from their next turn.`)
    deps.flashFooter(`Skills revision ${published.revision} published`, "info")
  }

  const run = async (args: readonly string[]) => {
    const [action = "list", ...rest] = args
    const flags = parseFlags(rest)
    const [first, second] = flags.positional
    switch (action) {
      case "list": case "ls": return showCatalog(["chariox", "project"])
      case "chariox": return showCatalog(["chariox"])
      case "project": case "environment": requireProject(); return showCatalog(["project"])
      case "from": return showFrom()
      case "show": return show(first)
      case "show-from": return showFromDetails(first)
      case "confirm": return confirm()
      case "cancel":
        review = null
        return deps.flashFooter("Skills review discarded", "info")
    }
    const result = await catalog()
    switch (action) {
      case "install": {
        if (!first) throw new SkillsCommandError("usage: /skills install <folder|url> [--project] [--revision <rev>] [--rename <name> | --replace <skill>] [--package <package>]")
        const scope = flags.project ? requireProject() : { kind: "chariox" as const }
        const packageRef = flags.values.get("package")
        return stage(result, scope, [{ action: "install", source: sourceFrom(first, flags.values.get("revision") ?? null),
          collision: collision(result, flags), package_id: packageRef ? findPackage(result, packageRef).id : null }])
      }
      case "repo": {
        if (!first) throw new SkillsCommandError("usage: /skills repo <https-url> [--project] [--name <package>] [--only <path,path>]")
        const scope = flags.project ? requireProject() : { kind: "chariox" as const }
        deps.flashFooter("Fetching the repository", "info")
        const listing = await request(skillsV2Request({ action: "resolve_repository", url: first }), "repository")
        const only = flags.values.get("only")?.split(",").map(path => path.trim().replace(/^\/+|\/+$/g, ""))
        const chosen = only ? listing.skills.filter(skill => only.includes(skill.path)) : listing.skills
        const lines = [`Repository ${listing.locator} @ ${listing.revision}: ${listing.skills.length} skill(s)`]
        for (const skill of listing.skills) lines.push(`  $${skill.name}  ${skill.path || "(repository root)"}${chosen.includes(skill) ? "" : " · not selected"}`)
        for (const skip of listing.skipped) lines.push(`  skipped ${skip.path || "(repository root)"}: ${skip.reason}`)
        deps.appendNotice(lines.join("\n"))
        const unknown = only?.filter(path => !listing.skills.some(skill => skill.path === path)) ?? []
        if (unknown.length) throw new SkillsCommandError(`the repository has no skill at ${unknown.join(", ")}; pick paths from the listing above or omit --only`)
        const packageId = randomUUID()
        const packageName = flags.values.get("name") ?? (listing.locator.replace(/\/+$/, "").split("/").pop() ?? "repository").replace(/\.git$/, "")
        return stage(result, scope, [
          { action: "create_package", package_id: packageId, name: packageName },
          ...chosen.map((skill): SkillCatalogAction => ({
            action: "install",
            source: { kind: "repository", url: first, revision: listing.revision, path: skill.path },
            collision: { action: "reject" },
            package_id: packageId,
          })),
        ])
      }
      case "import": {
        const entry = findNative(result, first)
        const scope = flags.project ? requireProject() : { kind: "chariox" as const }
        return stage(result, scope, [{ action: "install", source: { kind: "native", selection: entry.selection },
          collision: collision(result, flags), package_id: null }])
      }
      case "update": {
        const row = findSkill(result, first)
        const source = second ? sourceFrom(second, flags.values.get("revision") ?? null) : recordedSource(row)
        if (!source) throw new SkillsCommandError(`$${row.skill.name} has no recorded update source: give a folder or URL`)
        return stage(result, skillScope(row, false), [{ action: "update", skill_id: row.skill.id, source, expected_digest: row.skill.digest }])
      }
      case "remove": {
        const row = findSkill(result, first)
        return stage(result, skillScope(row, false), [{ action: "remove", skill_id: row.skill.id }])
      }
      case "state": {
        const row = findSkill(result, first)
        return stage(result, skillScope(row, flags.project), [{ action: "set_state", skill_id: row.skill.id, state: parseState(second) }])
      }
      case "create-package": {
        const name = flags.positional.join(" ")
        if (!name) throw new SkillsCommandError("name the package")
        return stage(result, flags.project ? requireProject() : { kind: "chariox" }, [{ action: "create_package", package_id: randomUUID(), name }])
      }
      case "move-to-package": {
        const row = findSkill(result, first)
        return stage(result, skillScope(row, false), [{ action: "regroup", skill_id: row.skill.id, package_id: findPackage(result, second).id }])
      }
      case "ungroup": {
        const row = findSkill(result, first)
        return stage(result, skillScope(row, false), [{ action: "regroup", skill_id: row.skill.id, package_id: null }])
      }
      case "package-state": {
        const record = findPackage(result, first)
        const scope = flags.project || record.scope.kind === "project" ? requireProject() : { kind: "chariox" as const }
        return stage(result, scope, [{ action: "set_package_state", package_id: record.id, state: parseState(second) }])
      }
      case "repo-update": case "update-package": {
        const record = findPackage(result, first)
        if (!record.custom && record.source_locator) {
          deps.flashFooter("Checking the repository's current commit", "info")
          const update = await request(skillsV2Request({ action: "resolve_repository_update", package_id: record.id }), "repository_update")
          const lines = [`Repository ${record.name}: ${update.from_revision} → ${update.revision}`]
          for (const changed of update.changed) lines.push(`  changed $${changed.member.name} · ${changed.member.path} · ${changed.member.digest.slice(0, 12)} → ${changed.digest.slice(0, 12)}`)
          for (const added of update.added) lines.push(`  new $${added.name} · ${added.path}`)
          for (const removed of update.removed) lines.push(`  remove $${removed.name} · ${removed.path}`)
          for (const unchanged of update.unchanged) lines.push(`  unchanged $${unchanged.name} · ${unchanged.path}`)
          for (const skipped of update.skipped) lines.push(`  skipped ${skipped.path}: ${skipped.reason}`)
          deps.appendNotice(lines.join("\n"))
          if (!update.mutation.actions.length) {
            deps.flashFooter(update.revision === update.from_revision ? "Repository package is up to date" : "No eligible package members to update", "info")
            return
          }
          return stage({ ...result, revision: update.mutation.expected_revision }, update.mutation.scope, update.mutation.actions)
        }
        if (action === "repo-update") throw new SkillsCommandError("this is a custom package; use /skills update-package or update its members individually")
        const members = result.skills.filter(row => row.skill.package_id === record.id)
        const missing = members.filter(row => !recordedSource(row))
        if (!members.length) throw new SkillsCommandError(`package ${record.name} has no skills`)
        if (missing.length) throw new SkillsCommandError(`no recorded update source for ${missing.map(row => row.skill.name).join(", ")}: update each with /skills update <skill> <folder|url>`)
        return stage(result, record.scope, members.map(row => ({ action: "update", skill_id: row.skill.id, source: recordedSource(row)!, expected_digest: row.skill.digest })))
      }
      case "remove-package": case "ungroup-package": {
        const record = findPackage(result, first)
        return stage(result, record.scope, [{ action: "remove_package", package_id: record.id, remove_skills: action === "remove-package" }])
      }
    }
    throw new SkillsCommandError(USAGE)
  }

  return async args => {
    try {
      await run(args)
    } catch (error) {
      const raw = error instanceof Error ? error.message : String(error)
      // The footer cuts long text; the transcript keeps the whole reason, without the transport wrappers.
      const reason = raw.replace(/^(?:(?:kernel|local) transport `[^`]*` failed: )+/, "")
      const message = error instanceof SkillsCommandError ? raw
        : /^timed out$/.test(reason) ? `Skills: ${UNANSWERED}`
          : `Skills: ${reason}`
      deps.appendNotice(message)
      deps.flashFooter(message, "error")
    }
  }
}

function formatReview(mutation: SkillCatalogMutation, preview: Preview): string {
  const lines = [
    `Review Skills changes (${mutation.scope.kind === "chariox" ? "Chariox" : "Project"}, expected revision ${mutation.expected_revision}): ${describeActions(mutation.actions)}`,
  ]
  for (const skill of preview.affected) {
    lines.push(`  $${skill.name} · ${skill.digest.slice(0, 12)} · ${skill.provenance.locator || skill.provenance.kind} ${skill.provenance.source_revision ?? ""}`.trimEnd())
    for (const file of skill.files) lines.push(`      ${file.path} · ${file.bytes} bytes${file.executable ? " · executable" : ""}`)
  }
  if (preview.collisions.length) lines.push(`  name collisions: ${preview.collisions.join(", ")}`)
  lines.push(`Review expires ${new Date(preview.review_receipt.expires_at_ms).toISOString()}. /skills confirm publishes all of it together; /skills cancel discards it.`)
  return lines.join("\n")
}

// A repository import is one package plus N installs: name them rather than repeating "install".
function describeActions(actions: readonly SkillCatalogAction[]): string {
  const repositoryInstalls = actions.filter(action => action.action === "install" && action.source.kind === "repository")
  const others = actions.filter(action => !repositoryInstalls.includes(action))
  const parts = others.map(action => (action.action === "create_package" ? `create_package ${action.name}` : action.action))
  const first = repositoryInstalls[0]
  if (first?.action === "install" && first.source.kind === "repository") {
    parts.push(`install ×${repositoryInstalls.length} from ${first.source.url} @ ${first.source.revision}`)
  }
  return parts.join(", ")
}
