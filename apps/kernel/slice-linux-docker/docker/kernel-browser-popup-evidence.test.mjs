// MP-08/MP-11: private dispatch evidence; these fixtures supplement live drills.
import test from "node:test";
import assert from "node:assert/strict";
import { BrowserPopupEvidence } from "./kernel-browser-popup-evidence.mjs";

function fixture(limit) {
  const handlers = new Set();
  const connection = { send:async method=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'frame'}}}:method==='Page.createIsolatedWorld'?{executionContextId:1}:{}, subscribe: handler => { handlers.add(handler); return () => handlers.delete(handler); } };
  return { evidence: new BrowserPopupEvidence(limit), handlers,
    browser: { ensureConnection: async () => connection,ensureTargetSession:async(_connection,target)=>target },
    create(targetId, openerId) {
      for (const handler of handlers) handler({method:"Target.targetCreated",params:{targetInfo:{type:"page",targetId,openerId}}});
    } };
}

test("MP-08 only the source's actual CDP dispatch window binds a creation", async () => {
  const {evidence,browser,handlers,create} = fixture();
  await evidence.capture(browser, {target_id:"source"}, "first-action", async dispatch => {
    create("before-input", "source");
    const end = dispatch();
    create("popup", "source");
    create("other-page", "other");
    end();
    create("native-after-input", "source");
  });
  assert.equal(handlers.size, 1);
  // Creation can precede the inventory that first adopts its target.
  assert.deepEqual(evidence.inventory([]), {});
  const tabs = ["popup", "before-input", "other-page", "native-after-input"].map(id => ({target_id:id,tab_id:id}));
  assert.deepEqual(evidence.inventory(tabs), {popup:"first-action"});
  await evidence.capture(browser, {target_id:"source"}, "second-action", async dispatch => {
    const end = dispatch(); create("second-popup", "source"); end();
  });
  assert.deepEqual(evidence.inventory([...tabs,{target_id:"second-popup",tab_id:"second"}]), {popup:"first-action",second:"second-action"});
  assert.deepEqual(evidence.inventory([]), {});
  assert.equal(evidence.targets.size, 0);
});

test("MP-08 concurrent source dispatches keep distinct creation identities", async () => {
  const {evidence,browser,create} = fixture();
  let ready,release;
  const started = new Promise(resolve => { ready=resolve; });
  const held = new Promise(resolve => { release=resolve; });
  const a = evidence.capture(browser, {target_id:"a"}, "action-a", async dispatch => {
    const end=dispatch();ready();await held;end();
  });
  await started;
  await evidence.capture(browser, {target_id:"b"}, "action-b", async dispatch => {
    const end=dispatch();create("popup-a","a");create("popup-b","b");end();
  });
  release();await a;
  assert.deepEqual(evidence.inventory([{target_id:"popup-a",tab_id:"a"},{target_id:"popup-b",tab_id:"b"}]), {a:"action-a",b:"action-b"});
});

test("MP-11 failed dispatch, bounded pending evidence and restart cannot retain a stale actor", async () => {
  const {evidence,browser,handlers,create} = fixture(2);
  await assert.rejects(evidence.capture(browser, {target_id:"source"}, "failed", async dispatch => {
    dispatch();create("failed-popup","source");throw Error("cancelled");
  }), /cancelled/);
  assert.equal(evidence.targets.size, 0);
  assert.equal(handlers.size, 1);
  await evidence.capture(browser, {target_id:"source"}, "completed", async dispatch => {
    const end=dispatch();for(const id of ["one","two","three"])create(id,"source");end();
  });
  assert.equal(evidence.targets.size, 2);
  evidence.clear();assert.equal(evidence.targets.size, 0);assert.equal(handlers.size,0);
});

test('MP-08 delayed creation follows the input reply, while later native input cancels activation evidence',async()=>{
 const {evidence,browser,create,handlers}=fixture();
 await evidence.capture(browser,{target_id:'source'},'async-agent',async dispatch=>{const end=dispatch();end()});
 for(const fn of handlers)fn({method:'Page.windowOpen',sessionId:'source',params:{url:'https://public.example/popup',userGesture:true}});
 create('delayed-popup','source');
 assert.deepEqual(evidence.inventory([{target_id:'delayed-popup',tab_id:'delayed'}]),{delayed:'async-agent'});
 for(const fn of handlers)fn({method:'Runtime.bindingCalled',sessionId:'source',params:{name:'charioxPopupNativeInput',payload:''}});
 for(const fn of handlers)fn({method:'Page.windowOpen',sessionId:'source',params:{url:'https://public.example/human',userGesture:true}});
 create('human-popup','source');
 assert.deepEqual(evidence.inventory([{target_id:'delayed-popup',tab_id:'delayed'},{target_id:'human-popup',tab_id:'human'}]),{delayed:'async-agent'});
 evidence.clear();assert.equal(handlers.size,0);
});
