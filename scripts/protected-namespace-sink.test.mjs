import test from "node:test"
import assert from "node:assert/strict"
import {verifyNamespaceAnchorDocuments, matchesVerifiedHostAncestor} from "../apps/kernel/slice-linux-docker/protected-namespace-entry.mjs"
const fixture = () => ({daemonUid:997,ancestors:[
  ["/",0,0o755], ["/var",0,0o755], ["/var/lib",0,0o755],
  ["/var/lib/chariox-docker",997,0o700], ["/var/lib/chariox-docker/private-layout",997,0o711],
  ["/var/lib/chariox-slice-share",0,0o710], ["/var/lib/chariox-slice-share/.broker-private",0,0o711],
  ["/var/lib/chariox-slice-share/.broker-private/artifacts",997,0o700],
].map(([path,hostUid,mode],index)=>({path,hostUid,mode,dev:"7",ino:String(400+index)}))})

test("installed archive sink requires host-attested ancestry, not only private-layout ancestry",()=>{
  const receipt=fixture()
  assert.equal(verifyNamespaceAnchorDocuments(receipt),receipt)
  for(const mutate of [r=>r.ancestors.splice(5),r=>r.ancestors[5].hostUid=997,r=>r.ancestors[6].mode=0o777,
    r=>r.ancestors[7].hostUid=0,r=>r.ancestors[7].mode=0o711,r=>r.ancestors[5].path="/unproven",
    r=>r.ancestors.push({...r.ancestors[0]}),r=>r.ancestors[5].ino="invalid"]){
    const changed=fixture();mutate(changed);assert.throws(()=>verifyNamespaceAnchorDocuments(changed))
  }
})
test("mapped sink ancestors match exact namespace-visible owner, mode and inode",()=>{
  const receipt=verifyNamespaceAnchorDocuments(fixture())
  for(const anchor of receipt.ancestors.filter(a=>a.path.includes("slice-share")&&a.hostUid===0)){
    const metadata={uid:65534,mode:anchor.mode,dev:BigInt(anchor.dev),ino:BigInt(anchor.ino)}
    assert.equal(matchesVerifiedHostAncestor(receipt,anchor.path,metadata),true)
    for(const changed of [{...metadata,uid:0},{...metadata,uid:1001},{...metadata,mode:0o777},
      {...metadata,ino:metadata.ino+1n},{...metadata,dev:metadata.dev+1n}]){
      assert.equal(matchesVerifiedHostAncestor(receipt,anchor.path,changed),false)
    }
    assert.equal(matchesVerifiedHostAncestor(receipt,`${anchor.path}/foreign`,metadata),false)
  }
})
