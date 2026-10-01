import assert from "node:assert/strict"
import test from "node:test"
import { observeRoomGestureFixture, roomCompanionGestureTargets, roomGestureObservationScript } from "./room-companion-gesture-fixture.mjs"

const fixtureOrigin = "http://host.docker.internal:23456"
const initial = { origin: {x:14,y:98}, windowGeometry:[12,0,1256,800] }
const current = { windowGeometry:[12,0,1256,800], selection:{x:160,y:40,width:800,height:100}, scroller:{x:160,y:420,width:960,height:160} }
// Exact RoomEnvironmentUpdated.environment.viewport shape from the kernel.
const viewport = {css_width:1280,css_height:800,device_scale_factor:1,
  desktop_pixel_width:1280,desktop_pixel_height:800,revision:1,last_actor_id:null}
test("canonical kernel viewport and native origin produce desktop gesture targets", () => {
  assert.deepEqual(roomCompanionGestureTargets(initial,current,viewport), {
    drag:{start:{x:254,y:188},end:{x:894,y:188}},scroll:{x:654,y:598},
  })
})
test("fresh DOM placement is used after reset", () => {
  const moved = {...current, selection:{...current.selection,y:80},scroller:{...current.scroller,y:450}}
  assert.equal(roomCompanionGestureTargets(initial,moved,viewport).drag.start.y,228)
  assert.equal(roomCompanionGestureTargets(initial,moved,viewport).scroll.y,628)
})
for (const [name,origin,now,screen] of [
  ["missing trusted origin", {...initial,origin:null},current,viewport],
  ["moved window", initial,{...current,windowGeometry:[13,0,1256,800]},viewport],
  ["unknown window geometry", {...initial,windowGeometry:[]},current,viewport],
  ["nonfinite origin", {...initial,origin:{x:NaN,y:98}},current,viewport],
  ["missing selection", initial,{...current,selection:null},viewport],
  ["empty selection", initial,{...current,selection:{...current.selection,width:0}},viewport],
  ["offscreen scroll", initial,{...current,scroller:{...current.scroller,y:800}},viewport],
]) test(name, () => assert.throws(() => roomCompanionGestureTargets(origin,now,screen)))

function connection(targets, value) {
  const calls=[]
  return {calls, async send(method,params,session) {
    calls.push({method,params,session})
    if(method==="Target.getTargets") return {targetInfos:targets}
    if(method==="Target.attachToTarget") return {sessionId:"fixture-session"}
    if(method==="Runtime.evaluate") return {result:{value}}
    if(method==="Target.detachFromTarget") return {}
    throw Error("unexpected CDP command")
  }}
}
const target={type:"page",targetId:"fixture",url:fixtureOrigin+"/click"}
const observation={...initial,...current,fixtureOrigin,fixturePath:"/click"}
test("observer chooses exact fixture tab and reads geometry without dispatching input", async () => {
  const c=connection([{...target,targetId:"other",url:"https://example.invalid/click"},target],observation)
  assert.deepEqual(await observeRoomGestureFixture(c,fixtureOrigin),observation)
  assert.deepEqual(c.calls.map(c=>c.method),["Target.getTargets","Target.attachToTarget","Runtime.evaluate","Target.detachFromTarget"])
  assert.equal(c.calls[1].params.targetId,"fixture")
  assert.equal(c.calls[2].params.returnByValue,true)
  assert.equal(c.calls[2].session,"fixture-session")
  assert.equal(c.calls[3].params.sessionId,"fixture-session")
})
test("ambiguous exact fixture tabs fail before attachment", async () => {
  const c=connection([target,{...target,targetId:"duplicate"}],observation)
  await assert.rejects(observeRoomGestureFixture(c,fixtureOrigin),/exactly one/)
  assert.equal(c.calls.length,1)
})
test("navigation races fail and detach", async () => {
  const c=connection([target],{...observation,fixturePath:"/other"})
  await assert.rejects(observeRoomGestureFixture(c,fixtureOrigin),/navigated/)
  assert.equal(c.calls.at(-1).method,"Target.detachFromTarget")
})
test("emitted observer uses immutable native slice helper and exact fixture origin", () => {
  const script=roomGestureObservationScript(fixtureOrigin)
  assert.ok(script.includes(observeRoomGestureFixture.toString()))
  assert.ok(script.includes('"/opt/chariox-slice/browser-controller-cdp.mjs"'))
  assert.throws(()=>roomGestureObservationScript("https://example.invalid"))
})
