// MP-08 / MP-10 / MP-11: shared conditional summary and snapshot-bound proposal pages.
import type {ProjectEnvironment, EnvironmentProposal} from "./kernel-types-project-environment-aggregate.js"
export type EnvironmentViewRead = {readonly knownDigest: string | null; readonly proposals: {readonly snapshotDigest:string;readonly offset:number;readonly limit:number} | null}
export type EnvironmentReadView = {
  readonly observed_at_ms:number; readonly snapshot_digest:string; readonly environment:ProjectEnvironment | null;
  readonly proposal_count:number; readonly proposal_offset:number; readonly next_proposal_offset:number | null;
  readonly proposals:readonly EnvironmentProposal[]; readonly operation_item_counts:readonly number[];
}
export type EnvironmentViewMetadata = Omit<EnvironmentReadView,"environment"|"proposals"> & {readonly received_at_ms?:number;readonly received_at_monotonic_ms?:number;readonly proposal_page_ids?:readonly string[]}
export function getProjectEnvironmentViewRequest(projectId:string, knownDigest:string | null = null) {
  return {GetProjectEnvironment:{projectId,view:{knownDigest,proposals:null}}} as const
}
export function getEnvironmentProposalPageRequest(projectId:string,snapshotDigest:string,offset:number,limit=20) {
  return {GetProjectEnvironment:{projectId,view:{knownDigest:null,proposals:{snapshotDigest,offset,limit}}}} as const
}
export function environmentFromView(projectId:string,value:unknown,previous:ProjectEnvironment | null):ProjectEnvironment {
  const reply=value as {ProjectEnvironmentView?:{view?:EnvironmentReadView};ProjectEnvironment?:{environment:ProjectEnvironment}}
  // The complete read remains supported for explicitly older clients and full mutation replies.
  if(reply.ProjectEnvironment?.environment)return reply.ProjectEnvironment.environment
  const view=reply.ProjectEnvironmentView?.view
  if(!view || !view.snapshot_digest || !Number.isSafeInteger(view.proposal_count) || view.proposal_count<0)throw new Error("Environment summary unavailable")
  const summary=view.environment ?? (previous?.read_view?.snapshot_digest===view.snapshot_digest ? previous : null)
  if(!summary || summary.local_project_id!==projectId || summary.schema_version!==1)throw new Error("Environment unavailable for this Project")
  const {environment:_,proposals,...read_view}=view
  return {...summary,proposals,read_view:{...read_view,received_at_ms:Date.now(),received_at_monotonic_ms:performance.now(),proposal_page_ids:proposals.map(p=>p.proposal_id)}}
}
export function createEnvironmentReader(send:(request:unknown)=>Promise<unknown>) {
  const snapshots=new Map<string,ProjectEnvironment>()
  return async(projectId:string):Promise<ProjectEnvironment>=>{
    const previous=snapshots.get(projectId) ?? null
    try {
      const environment=environmentFromView(projectId,await send(getProjectEnvironmentViewRequest(projectId,previous?.read_view?.snapshot_digest ?? null)),previous)
      snapshots.delete(projectId);snapshots.set(projectId,environment)
      if(snapshots.size>8)snapshots.delete(snapshots.keys().next().value!)
      return environment
    }catch(error){snapshots.delete(projectId);throw error}
  }
}
