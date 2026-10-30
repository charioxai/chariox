// MP-08 / MP-10 / MP-11: conditional reads never accept another kernel's cached state.
import assert from "node:assert/strict"
import test from "node:test"
import {createEnvironmentReader,environmentFromView,getEnvironmentProposalPageRequest} from "./project-environment-read-view.js"
import type {ProjectEnvironment} from "./kernel-types-project-environment-aggregate.js"
const environment={schema_version:1,local_project_id:"p",project_requirements:[],folders:[],proposals:[],operations:[]} as unknown as ProjectEnvironment
const view={observed_at_ms:Date.now(),snapshot_digest:"one",environment,proposal_count:1000,proposal_offset:0,next_proposal_offset:8,proposals:[],operation_item_counts:[]}
test("compact reads revalidate every opening and omit unchanged sections",async()=>{
 const requests:any[]=[]
 const read=createEnvironmentReader(async request=>{requests.push(request);return {ProjectEnvironmentView:{view:requests.length===1 ? view : {...view,environment:null}}}})
 const first=await read("p"),next=await read("p")
 assert.equal(first.read_view?.proposal_count,1000)
 assert.equal(next.project_requirements,first.project_requirements)
 assert.equal(requests[0].GetProjectEnvironment.view.knownDigest,null)
 assert.equal(requests[1].GetProjectEnvironment.view.knownDigest,"one")
 assert.deepEqual(getEnvironmentProposalPageRequest("p","one",8),{GetProjectEnvironment:{projectId:"p",view:{knownDigest:null,proposals:{snapshotDigest:"one",offset:8,limit:20}}}})
})
test("missing or changed conditional summary cannot borrow stale or cross-Project state",()=>{
 assert.throws(()=>environmentFromView("p",{ProjectEnvironmentView:{view:{...view,environment:null}}},null),/unavailable/)
 const first=environmentFromView("p",{ProjectEnvironmentView:{view}},null)
 assert.throws(()=>environmentFromView("other",{ProjectEnvironmentView:{view:{...view,environment:null}}},first),/unavailable/)
 assert.throws(()=>environmentFromView("p",{ProjectEnvironmentView:{view:{...view,environment:null,snapshot_digest:"changed"}}},first),/unavailable/)
})

test("bounded reads reject a full response without the requested snapshot metadata",async()=>{
 const read=createEnvironmentReader(async()=>({ProjectEnvironment:{environment}}))
 await assert.rejects(()=>read("p"),/Environment summary unavailable/)
})
