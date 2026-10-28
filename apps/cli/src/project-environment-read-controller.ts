import { environmentReadFailureMessage } from "./project-environment-read-errors.js"
// MP-08 / MP-10 / MP-11: Detect uses shared request builders.
import { projectEnvironmentOperationRequest, relayStatusRequest, cloudRelayStatusRequest } from "@chariox/kernel-client/ipc-requests"
// MP-08 / MP-10: Project read-only navigation, independent of session selection.
import { createEnvironmentReader, getEnvironmentProposalPageRequest, type EnvironmentReadView, createEnvironmentViewCache, projectEnvironmentLines, environmentOriginLabel, environmentRevisionDraft, saveEnvironmentRevisionRequest, environmentDiffLines, environmentSnapshotDiff, type EnvironmentRevisionDiff, type ProjectEnvironment, checkEnvironmentRequest, environmentObservationExpiry } from "@chariox/kernel-client"

export function projectEnvironmentPageSize(height: number) {
  return Math.max(1, height - Math.max(1, Math.floor(height / 5)) - 4)
}

// Inner text width of the Environment panel (see cli-dialog-overlay.ts: panel width, 2-column padding each side).
export function projectEnvironmentPanelWidth(viewportWidth: number) {
  return Math.min(110, Math.max(30, viewportWidth - 4))
}

export function projectEnvironmentTextWidth(viewportWidth: number) {
  return Math.max(1, Math.min(projectEnvironmentPanelWidth(viewportWidth), viewportWidth - 4) - 4)
}

// Word-wrap keeping the line's indentation; words longer than the row are split.
function wrapLine(line: string, width: number): string[] {
  if (line.length <= width) return [line]
  const indent = " ".repeat(Math.min(line.length - line.trimStart().length, Math.floor(width / 2)))
  const room = width - indent.length
  const words = line.trimStart().split(" ").flatMap(word => word.match(new RegExp(`.{1,${room}}`, "gs")) ?? [""])
  const rows: string[] = []
  for (const word of words) {
    const last = rows.at(-1)
    if (last !== undefined && last.length + 1 + word.length <= room) rows[rows.length - 1] = `${last} ${word}`
    else rows.push(word)
  }
  return rows.map(row => indent + row)
}

export function createProjectEnvironmentReadController(deps: {
  send: (request: unknown) => Promise<unknown>
  render: () => void
  pageSize: () => number
  textWidth: () => number
  copyLink?: (link: string) => void
}) {
  const cache = createEnvironmentViewCache()
  let previousProposalOffsets: number[] = []
  const readEnvironment = createEnvironmentReader(deps.send)
  const scope = Symbol("kernel-client-view")
  let refreshing = false
  let open = false
  let generation = 0
  let lines: string[] = []
  let offset = 0
  const displayLines = () => lines.flatMap(line => wrapLine(line, deps.textWidth()))
  let requestedProjectId: string | null = null
  let current: ProjectEnvironment | null = null
  let proposalIndex = 0
  let diff: EnvironmentRevisionDiff | null = null
  let showingDiff = false
  let diffLabel = "Kernel Save diff"
  let webLink: string | null = null
  let savingGeneration: number | null = null
  let detectionGeneration: number | null = null
  let checkingGeneration: number | null = null
  let expiryTimer: ReturnType<typeof setTimeout> | undefined
  const controller = {
    isOpen: () => open,
    visibleLines: () => displayLines().slice(offset, offset + Math.max(1, deps.pageSize())),
    close() { clearTimeout(expiryTimer); open = false; generation++; deps.render() },
    async open(projectId: string) {
      previousProposalOffsets = []
      const requestGeneration = ++generation
      const prior = cache.peek(scope, projectId)
      if (current?.local_project_id !== projectId) diff = null
      requestedProjectId = projectId; open = true; refreshing = true; showingDiff = false; webLink = null; current = cache.peek(scope, projectId); proposalIndex = 0; offset = 0
      lines = current ? ["Refreshing Environment… · last kernel snapshot", ...viewLines({...current,observations:(current.observations??[]).map(o=>({...o,stale:true}))})] : ["Loading Environment…"]; deps.render()
      try {
        const environment = await readEnvironment(projectId)
        if (!environment || environment.local_project_id !== projectId || environment.schema_version !== 1) throw new Error("Environment unavailable for this Project")
        if (requestGeneration !== generation) return
        if (prior) { const observed = environmentSnapshotDiff(prior, environment); if (observed) { diff = observed; diffLabel = "Observed revision changes · between kernel snapshots" } }
        cache.remember(scope, environment); current = environment
        lines = viewLines(environment)
      } catch (error) {
        if (requestGeneration !== generation) return
        cache.forget(scope, projectId); current = null
        lines = [environmentReadFailureMessage(error), "r · Refresh"]
      }
      if (requestGeneration === generation) refreshing = false
      deps.render()
    },
    async check() {
      if (!current || refreshing || savingGeneration === generation || detectionGeneration === generation || checkingGeneration === generation || !current.delivered_capabilities.enabled_environment_operations.includes("check")) return
      const g = generation, projectId = current.local_project_id
      checkingGeneration = g
      lines = ["Checking this machine… · no repair", ...viewLines(current)]; deps.render()
      try {
        const response = await deps.send(relayStatusRequest()) as {RelayStatus?: {status?: {machine_id:string;daemon_id:string}}}
        const status=response.RelayStatus?.status
        if(g!==generation)return
        if (!status) throw new Error("Kernel identity unavailable")
        const request=checkEnvironmentRequest(current,{machine_id:status.machine_id,target_instance_generation:status.daemon_id,slice_ref:null},`check-${crypto.randomUUID()}`)
        const ack=await deps.send(request) as {EnvironmentOperation?: {operation?: {operation_id:string;phase:string}}}
        if(g!==generation)return
        if (!ack.EnvironmentOperation?.operation)throw new Error("Check unavailable")
        for (let i=0;i<400 && g===generation;i++) {
          const snapshot=await readEnvironment(projectId)
          if (!snapshot || snapshot.local_project_id!==projectId) throw new Error("Check unavailable for this Project")
          if(g!==generation)return
          cache.remember(scope,snapshot); current=snapshot; lines=viewLines(snapshot); deps.render()
          // MP-08 / MP-10 / MP-11: another client can replace the aggregate
          // projection; retained acknowledgements use the shared kernel lookup.
          const op=snapshot.operations.find(o=>o.operation_id===request.CheckProjectEnvironment.operationId)
            ?? (await deps.send(projectEnvironmentOperationRequest("GetEnvironmentOperation",{projectId,operationId:request.CheckProjectEnvironment.operationId})) as {EnvironmentOperation?:{operation?:ProjectEnvironment["operations"][number]}}).EnvironmentOperation?.operation
          if(g!==generation)return
          if(!op || op.operation_id!==request.CheckProjectEnvironment.operationId)throw new Error("Check operation unavailable")
          if(op && ["ready","failed","cancelled"].includes(op.phase))return
          await new Promise(resolve=>setTimeout(resolve,250))
        }
        if(g===generation)throw new Error("Check polling deadline exceeded")
      } catch {if(g===generation)lines=["Check unavailable · refresh and try again",...viewLines(current!)]}
      finally {if(checkingGeneration===g)checkingGeneration=null;if(g===generation)deps.render()}
    },
    async detect() {
      if (!current || refreshing || detectionGeneration === generation || checkingGeneration === generation || !current.delivered_capabilities.enabled_environment_operations.includes("detect")) return
      const requestGeneration = generation
      const projectId = current.local_project_id
      detectionGeneration = requestGeneration
      lines = ["Detecting requirements…"]; deps.render()
      try {
        const statusResponse = await deps.send(relayStatusRequest()) as { RelayStatus?: { status: { machine_id: string; daemon_id: string } } }
        const status = statusResponse.RelayStatus?.status
        if (!status?.machine_id || !status.daemon_id) throw new Error("Kernel identity unavailable")
        if (requestGeneration !== generation) return
        const response = await deps.send(projectEnvironmentOperationRequest("DetectProjectEnvironment", { projectId, operationId: `detect-${crypto.randomUUID()}`, folderIds: [], target: { machine_id: status.machine_id, target_instance_generation: status.daemon_id, slice_ref: null }, provider: null, allowModelFolders: [] })) as { ProjectEnvironment?: { environment: ProjectEnvironment } }
        const environment = response.ProjectEnvironment?.environment
        if (!environment || environment.local_project_id !== projectId) throw new Error("Detect unavailable for this Project")
        if (requestGeneration !== generation) return
        cache.remember(scope, environment); current = environment; previousProposalOffsets = []; proposalIndex = 0; lines = viewLines(environment); offset = 0
      } catch (error) {
        if (requestGeneration !== generation) return
        lines = [error instanceof Error ? error.message : "Detect unavailable", ...viewLines(current!)]
      } finally { if (detectionGeneration === requestGeneration) detectionGeneration = null; if (requestGeneration === generation) deps.render() }
    },
    async web() {
      if (!current || refreshing) return
      const g = generation, projectId = current.local_project_id
      try {
        const [response, relay] = await Promise.all([deps.send(cloudRelayStatusRequest()), deps.send(relayStatusRequest())]) as [{ CloudRelayStatus?: { profile?: { api_url?: string } } }, { RelayStatus?: { status?: { daemon_id?: string } } }]
        if (g !== generation) return
        const api = response.CloudRelayStatus?.profile?.api_url
        const kernelId = relay.RelayStatus?.status?.daemon_id
        if (!kernelId) throw new Error("Kernel identity unavailable for its Web Environment link")
        if (!api) throw new Error("Connect this kernel to Cloud to obtain its Web Environment link")
        const origin = new URL(api)
        if (!["https:", "http:"].includes(origin.protocol) || origin.username || origin.password) throw new Error("Web Environment origin unavailable")
        const link = new URL("/waiting-room", origin.origin)
        link.searchParams.set("environmentProjectId", projectId)
        link.searchParams.set("environmentKernelId", kernelId)
        webLink = link.href; deps.copyLink?.(webLink); lines = viewLines(current); offset = 0
      } catch (error) { if (g === generation) lines = [error instanceof Error ? error.message : "Web link unavailable", ...viewLines(current!)] }
      if (g === generation) deps.render()
    },
    async review(decision: "accept" | "exclude") {
      if (!current || refreshing || savingGeneration === generation || detectionGeneration === generation || checkingGeneration === generation || !current.delivered_capabilities.enabled_environment_operations.includes("save")) return
      const proposal = current.proposals[proposalIndex]
      if (!proposal) return
      const requestGeneration = generation
      previousProposalOffsets = []
      savingGeneration = requestGeneration
      let savedReply: ProjectEnvironment | null = null
      try {
        const response = await deps.send(saveEnvironmentRevisionRequest(current, environmentRevisionDraft(current), decision === "accept" ? [proposal.proposal_id] : [], decision === "exclude" ? [proposal.proposal_id] : [])) as { ProjectEnvironmentSaved?: { environment: ProjectEnvironment; diff?: EnvironmentRevisionDiff } }
        if (generation !== requestGeneration) return
        const saved = response.ProjectEnvironmentSaved?.environment
        if (!saved || saved.local_project_id !== current.local_project_id) throw new Error("Proposal review unavailable")
        savedReply = saved
        // MP-08 / MP-10 / MP-11: unchanged-content decisions preserve Check
        // freshness by reading a clock-bearing snapshot from this kernel.
        const fresh = current.read_view && !saved.read_view ? await readEnvironment(saved.local_project_id) : saved
        if (generation !== requestGeneration) return
        if (fresh.local_project_id !== saved.local_project_id || fresh.lineage.environment_id !== saved.lineage.environment_id || fresh.revision < saved.revision || (current.read_view && !fresh.read_view)) throw new Error("Saved Environment refresh unavailable")
        diff = response.ProjectEnvironmentSaved?.diff ?? null; diffLabel = "Kernel Save diff"; showingDiff = false; cache.remember(scope, fresh); current = fresh; proposalIndex = Math.min(proposalIndex, Math.max(0, fresh.proposals.length - 1)); lines = viewLines(fresh); offset = 0
      } catch (error) {
        if (generation === requestGeneration) {
          if (savedReply) { cache.remember(scope,savedReply); current = savedReply; proposalIndex = 0 }
          lines = [savedReply ? "Proposal decision saved · Refresh to load current Check freshness" : error instanceof Error ? error.message : "Proposal review failed", "r · Refresh to review the latest revision", ...viewLines(current)]
        }
      } finally { if (savingGeneration === requestGeneration) savingGeneration = null; if (generation === requestGeneration) deps.render() }
    },
    async proposalPage(proposalOffset:number, selectLast=false) {
      if(!current?.read_view || refreshing || savingGeneration===generation || detectionGeneration===generation || checkingGeneration===generation)return
      const baseline=current, g=generation
      const oldOffset=baseline.read_view!.proposal_offset
      refreshing=true
      try {
        const response=await deps.send(getEnvironmentProposalPageRequest(baseline.local_project_id,baseline.read_view!.snapshot_digest,proposalOffset)) as {ProjectEnvironmentView?:{view:EnvironmentReadView}}
        if(g!==generation)return
        const view=response.ProjectEnvironmentView?.view
        if(!view || view.snapshot_digest!==baseline.read_view!.snapshot_digest || view.environment || view.proposal_offset!==proposalOffset)throw new Error("Environment changed · r Refresh to review")
        if(selectLast)previousProposalOffsets.pop();else if(proposalOffset>oldOffset)previousProposalOffsets.push(oldOffset)
        current={...baseline,proposals:view.proposals,read_view:{...baseline.read_view!,proposal_offset:proposalOffset,next_proposal_offset:view.next_proposal_offset}}
        proposalIndex=selectLast ? Math.max(0,current.proposals.length-1) : 0
        lines=viewLines(current);offset=0
      }catch(error){if(g===generation)lines=[error instanceof Error ? error.message : "Proposal page unavailable",...viewLines(baseline)]}
      finally{if(g===generation){refreshing=false;deps.render()}}
    },
    handleKey(event: { name: string; eventType?: string; ctrl?: boolean; alt?: boolean; meta?: boolean }) {
      if (!open) return false
      if (event.eventType === "release") return true
      if (event.ctrl && (event.name === "c" || event.name === "e")) return false
      if (event.ctrl || event.alt || event.meta) return true
      if (event.name === "w") { void controller.web(); return true }
      if (event.name === "v" && current) { showingDiff = !showingDiff; lines = showingDiff ? (diff ? [diffLabel, ...environmentDiffLines(diff)] : ["No revision diff from this client yet · review a proposal or open Web", ...viewLines(current)]) : viewLines(current); offset = 0; deps.render(); return true }
      if ((event.name === "left" || event.name === "right") && current) {
        const sections = [{ id: "project", title: "Project-wide" }, ...current.folders.map(f => ({ id: f.folder_id, title: f.label }))]
        let sourceOffset = -1
        const positions = sections.flatMap(section => {
          // Search in section order so folders with the same label keep distinct headings.
          const heading = lines.indexOf(section.title, sourceOffset + 1)
          if (heading < 0) return []
          sourceOffset = heading
          return [{ id: section.id, offset: lines.slice(0, heading).reduce((rows, line) => rows + wrapLine(line, deps.textWidth()).length, 0) }]
        }).map(section => section.offset)
        offset = event.name === "right" ? (positions.find(p => p > offset) ?? 0) : ([...positions].reverse().find(p => p < offset) ?? 0)
        deps.render(); return true
      }
      if (event.name === "r" && requestedProjectId) { void controller.open(requestedProjectId); return true }
      if (event.name === "a" || event.name === "x") { void controller.review(event.name === "a" ? "accept" : "exclude"); return true }
      if ((event.name === "n" || event.name === "p") && current && (current.proposals.length || (current.read_view?.proposal_count ?? 0)>0)) {
        if(refreshing)return true
        if(!current.proposals.length && current.read_view?.next_proposal_offset!=null){void controller.proposalPage(current.read_view.next_proposal_offset);return true}
        if(current.read_view && event.name==="n" && proposalIndex===current.proposals.length-1 && current.read_view.next_proposal_offset!=null){void controller.proposalPage(current.read_view.next_proposal_offset);return true}
        if(current.read_view && event.name==="p" && proposalIndex===0 && current.read_view.proposal_offset>0){void controller.proposalPage(previousProposalOffsets.at(-1) ?? 0,true);return true}
        proposalIndex = Math.max(0, Math.min(current.proposals.length - 1, proposalIndex + (event.name === "n" ? 1 : -1)))
        lines = viewLines(current); offset = 0; deps.render(); return true
      }
      if (event.name === "c") { void controller.check(); return true }
      if (event.name === "d") { void controller.detect(); return true }
      if (event.name === "escape") { controller.close(); return true }
      const amount = event.name === "down" ? 1 : event.name === "up" ? -1 : event.name === "pagedown" ? deps.pageSize() : event.name === "pageup" ? -deps.pageSize() : 0
      offset = Math.max(0, Math.min(Math.max(0, displayLines().length - deps.pageSize()), offset + amount))
      deps.render()
      return true
    },
  }
  function viewLines(environment: ProjectEnvironment): string[] {
    clearTimeout(expiryTimer)
    const deadline = environmentObservationExpiry(environment)
    if (deadline != null) {
      expiryTimer = setTimeout(() => {
        if (open && current === environment && !refreshing && !showingDiff && savingGeneration !== generation && detectionGeneration !== generation && checkingGeneration !== generation) {
          lines = viewLines(environment); deps.render()
        }
      }, Math.max(0, deadline - Date.now()))
      expiryTimer.unref?.()
    }
    const selected = environment.proposals[proposalIndex]
    return [`Revision ${environment.revision ?? 0} · r Refresh`, ...(environment.delivered_capabilities.enabled_environment_operations.includes("check") ? ["c · Check this machine (read-only)", ...environment.operations.filter(o=>o.kind==="check").slice(-1).map(o=>`Check · ${o.phase} · ${o.operation_id}`)] : ["Check is unavailable on this kernel"]), ...(environment.delivered_capabilities.enabled_environment_operations.includes("save") ? ["n/p · Next/previous proposal; a · Accept; x · Exclude", ...(environment.proposals[proposalIndex] ? [`Review ${(environment.read_view?.proposal_offset ?? 0)+proposalIndex + 1}/${environment.read_view?.proposal_count ?? environment.proposals.length} · ${selected!.requirement.title}`, selected!.requirement.spec.kind === "software" ? `Version constraint: ${selected!.requirement.spec.version_constraint ?? "Not specified"}` : `Kind: ${selected!.requirement.spec.kind}`, ...selected!.requirement.origins.map(environmentOriginLabel), "Field editing is available in the Project’s Web Environment."] : [environment.read_view?.proposal_count ? `${environment.read_view.proposal_count} proposals · n to begin review` : "No pending proposals"])] : []), ...(environment.delivered_capabilities.enabled_environment_operations.includes("detect") ? ["d · Detect requirements (read-only)", "File names and types from code-manifest folders are sent to your provider.", "Other folders use deterministic detection; model opt-in is available in Web."] : []), "←/→ · Project/folder navigation; v · Revision diff", "Typed field editing is temporarily Web-first · w · Copy Web Environment link", ...(webLink ? [webLink] : []), `Content · ${environment.content_digest ?? "Initial"} · Parent · ${environment.parent_revision_digest ?? "Initial revision"}`, ...projectEnvironmentLines(environment), "An Environment lists what this project needs on this machine.", environment.delivered_capabilities.enabled_environment_operations.includes("save") ? "Edit requirements in Web; this terminal supports Check and proposal review." : "Read only: this kernel can show Environments but cannot change them yet. Update Chariox on this machine to edit."]
  }
  return controller
}
