// MP-08 / MP-10 / MP-11: no dummy session or provider in the standalone contract.
import assert from "node:assert/strict"
import test from "node:test"
import { resolveProjectRequest, projectResolutionFromResponse, projectResolutionMinimumProtocolVersion, projectResolutionFailureMessage } from "./project-resolution.js"
test("standalone Project lookup and explicit creation are distinct", () => {
  assert.deepEqual(resolveProjectRequest("/src"), {ResolveProject:{workspace_id:"/src",project_id:null,create_if_missing:false}})
  assert.deepEqual(resolveProjectRequest("/src","stale",true), {ResolveProject:{workspace_id:"/src",project_id:"stale",create_if_missing:true}})
  assert.equal(projectResolutionMinimumProtocolVersion,661)
  assert.equal(projectResolutionFromResponse({ProjectResolved:{project:null}}),null)
  assert.throws(()=>projectResolutionFromResponse({}),/unavailable/)
})

test("old kernel diagnostics become a plain upgrade step",()=>{
 assert.equal(projectResolutionFailureMessage(Error("kernel too old: Project Environment setup needs protocol ≥524; this kernel is 505; update the kernel")),"Upgrade this kernel to set up Project Environments.")
})
