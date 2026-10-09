import assert from "node:assert/strict"
import test from "node:test"
import {captureVisibleRegionRequest,visibleRegionCaptureMinimumProtocolVersion} from "./ipc-screenshot-requests.js"
import {LOCAL_DAEMON_PROTOCOL_VERSION} from "./kernel-types.js"
test("443 captures a pinned visible surface without a caller identity or new prompt authority",()=>{
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION,481);assert.equal(visibleRegionCaptureMinimumProtocolVersion,443)
  const surface={kind:"kernel_browser" as const,tab_id:"host-tab-1",generation:2},region={x:1,y:2,width:3,height:4,viewport_width:640,viewport_height:400,frame_width:1280,frame_height:800}
  assert.deepEqual(captureVisibleRegionRequest("c",surface,region),{CaptureVisibleRegion:{capture_id:"c",surface,region}})
})
