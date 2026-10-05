import assert from "node:assert/strict"
import test from "node:test"
import {createHash} from "node:crypto"
import {readFile,stat,access} from "node:fs/promises"
import {withRegionScreenshot} from "./region-screenshot.js"
const png=Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aLX0AAAAASUVORK5CYII=","base64")
test("region capture verifies owner surface/digest and uses existing image file intake with private temporary cleanup",async()=>{
  let attachedPath="",requests:unknown[]=[]
  const send=async<T>(request:unknown):Promise<T>=>{
    requests.push(request)
    if((request as {KernelBrowser?:unknown}).KernelBrowser)return {KernelBrowser:{result:{generation:4,viewport:{css_width:1280,css_height:800},tabs:[{tab_id:"host-tab-1"}]}}} as T
    const input=(request as {CaptureVisibleRegion:{capture_id:string;surface:unknown;region:unknown}}).CaptureVisibleRegion
    assert.deepEqual(input.region,{x:0,y:0,width:1,height:1,viewport_width:1280,viewport_height:800,frame_width:1280,frame_height:800})
    return {VisibleRegionCaptured:{capture:{capture_id:input.capture_id,surface:input.surface,captured_at_ms:12,width:1,height:1,media_type:"image/png",sha256:createHash("sha256").update(png).digest("hex"),data_base64:png.toString("base64")}}} as T
  }
  await withRegionScreenshot({args:["tab","host-tab-1","0","0","1","1"],sessionId:"s",attachmentId:"a",send,check:()=>{},attach:async file=>{
    attachedPath=file.path;assert.equal(file.kind,"image");assert.equal(file.filename,"chariox-tab-host-tab-1-12.png");assert.deepEqual(await readFile(file.path),png);assert.equal((await stat(file.path)).mode&0o777,0o600)
  }})
  await assert.rejects(access(attachedPath),{code:"ENOENT"});assert.equal(requests.length,2)
})
test("invalid or changed prompt target creates no screenshot file or attachment",async()=>{
  const base={sessionId:"s",attachmentId:"a",send:async<T>():Promise<T>=>{throw new Error("should not send")},check:()=>{throw new Error("target changed")},attach:async()=>{assert.fail("should not attach")}}
  await assert.rejects(withRegionScreenshot({...base,args:["tab","t","0","0","1","1"]}),/target changed/)
  await assert.rejects(withRegionScreenshot({...base,args:["tab","t","-1","0","1","1"]}),/usage/)
})
