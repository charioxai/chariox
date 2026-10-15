// MP-08 / MP-10 / MP-11: kernel-clock age must not depend on the remote client's clock.
import test from "node:test"
import assert from "node:assert/strict"
import {environmentObservationStatus,environmentObservationExpiry} from "./project-environment-aggregate.js"
import type {ProjectEnvironment} from "./kernel-types-project-environment-aggregate.js"
test("review0255 current Check and expiry use kernel age across clock skew",()=>{
 const clientNow=Date.now(),kernelNow=clientNow+1000
 const base={schema_version:1,local_project_id:"p",revision:1,content_digest:"digest",lineage:{project_id:"p",environment_id:"env"},project_requirements:[{requirement_id:"req",title:"Req",scope:{kind:"project"},spec:{kind:"software"},depends_on:[]}],folders:[],proposals:[],operations:[],observations:[{environment_id:"env",requirement_id:"req",revision_digest:"digest",checked_at_ms:kernelNow-100,status:"ready",stale:false,target:{machine_id:"m",target_instance_generation:"g",slice_ref:null}}],read_view:{snapshot_digest:"snapshot",proposal_count:0,proposal_offset:0,next_proposal_offset:null,operation_item_counts:[],observed_at_ms:kernelNow,received_at_ms:clientNow}} as unknown as ProjectEnvironment
 assert.equal(environmentObservationStatus(base,"req"),"Ready")
 assert.equal(environmentObservationExpiry(base,clientNow),clientNow+299901)
 const ahead={...base,read_view:{...base.read_view!,observed_at_ms:clientNow-600000,received_at_ms:clientNow},observations:[{...base.observations[0]!,checked_at_ms:clientNow-600100}]}
 assert.equal(environmentObservationStatus(ahead,"req"),"Ready")
 assert.equal(environmentObservationExpiry(ahead,clientNow),clientNow+299901)
})
