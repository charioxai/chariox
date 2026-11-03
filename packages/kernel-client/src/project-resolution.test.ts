// MP-08 / MP-10 / MP-11: no dummy session or provider in the standalone contract.
import assert from "node:assert/strict"
import test from "node:test"
import { resolveProjectRequest, projectResolutionFromResponse } from "./project-resolution.js"
test("standalone Project lookup and explicit creation are distinct", () => {
  assert.deepEqual(resolveProjectRequest("/src"), {ResolveProject:{workspace_id:"/src",project_id:null,create_if_missing:false}})
  assert.deepEqual(resolveProjectRequest("/src","stale",true), {ResolveProject:{workspace_id:"/src",project_id:"stale",create_if_missing:true}})
  assert.equal(projectResolutionFromResponse({ProjectResolved:{project:null}}),null)
  assert.throws(()=>projectResolutionFromResponse({}),/unavailable/)
})
