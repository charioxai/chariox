import { createHash, randomUUID } from "node:crypto"
import { mkdtemp, writeFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { captureVisibleRegionRequest, type ScreenshotSurface, type VisibleRegionCapture } from "@chariox/kernel-client/ipc-requests"
import { sendWithProtocolMinimum } from "./protocol-minimum-diagnostic.js"

/** Terminal coordinate equivalent: desktop pixels, no inferred terminal-to-video
 * scale. Existing intake stores the verified PNG through StoreTransferredFile. */
export async function withRegionScreenshot(input: {
  args: string[]; sessionId: string; attachmentId: string
  send<T>(request: unknown): Promise<T>; check(): void
  attach(file: {path: string; filename: string; mime: string; kind: "image"}): Promise<void>
}): Promise<void> {
  const [kind, ...rest] = input.args
  const id = kind === "room" ? input.sessionId : rest.shift()
  if(!id || !["room","tab","app"].includes(kind??"") || rest.length!==4 || !rest.every(n=>/^\d+$/.test(n)))throw new Error("usage: /attach region room | tab <id> | app <id> <x> <y> <width> <height>")
  const [x,y,width,height]=rest.map(Number)
  if(![x,y,width,height].every(Number.isSafeInteger))throw new Error("Invalid capture coordinates")
  let surface: ScreenshotSurface, frameWidth: number, frameHeight: number
  input.check()
  if(kind === "room") {
    const state=await input.send<{RoomEnvironmentState?:{environment:{runtime_generation:number;viewport:{revision:number;desktop_pixel_width:number;desktop_pixel_height:number}}}}>({GetRoomEnvironmentState:{session_id:id}})
    const environment=state.RoomEnvironmentState?.environment
    if(!environment)throw new Error("Room environment is unavailable")
    surface={kind:"room",session_id:id,attachment_id:input.attachmentId,runtime_generation:environment.runtime_generation,viewport_revision:environment.viewport.revision}
    frameWidth=environment.viewport.desktop_pixel_width;frameHeight=environment.viewport.desktop_pixel_height
  } else {
    const response=await input.send<{KernelBrowser?:{result:{generation:number;viewport:{css_width:number;css_height:number};tabs:{tab_id:string}[]}}}>({KernelBrowser:{command:{op:"state"}}})
    const state=response.KernelBrowser?.result
    if(!state)throw new Error("Kernel browser is unavailable")
    let tabId = id, generation = state.generation
    if(kind === "app") {
      const response=await input.send<{UserAppViewsListed?:{views:{view_id:string;browser?:{tab_id:string;generation:number}}[]}}>({ListUserAppViews:{}})
      const browser=response.UserAppViewsListed?.views.find(view=>view.view_id===id)?.browser
      if(!browser)throw new Error("App capture requires a kernel-hosted view; native TUI projection has no pixel surface")
      surface={kind:"user_app_view",view_id:id,generation:browser.generation}
      tabId=browser.tab_id;generation=browser.generation
    } else surface={kind:"kernel_browser",tab_id:id,generation:state.generation}
    // Display negotiation keeps a per-tab DPR even after the viewer closes.
    // The existing protected screenshot seam reports that tab's native pixels;
    // state.viewport describes CSS layout and cannot size a region attachment.
    const observed=await input.send<{KernelBrowser?:{result:{tab_id:string;generation:number;width:number;height:number}}}>({KernelBrowser:{command:{op:"screenshot",tab_id:tabId,generation}}})
    const frame=observed.KernelBrowser?.result
    if(!frame || frame.tab_id!==tabId || frame.generation!==generation
      || !Number.isSafeInteger(frame.width) || !Number.isSafeInteger(frame.height)
      || frame.width<=0 || frame.height<=0 || frame.width*frame.height>16*1024*1024)throw new Error("Native capture geometry is unavailable")
    frameWidth=frame.width;frameHeight=frame.height
  }
  const captureId=randomUUID()
  const response=await sendWithProtocolMinimum<{VisibleRegionCaptured?:{capture:VisibleRegionCapture}}>(input.send,captureVisibleRegionRequest(captureId,surface,{x:x!,y:y!,width:width!,height:height!,viewport_width:frameWidth,viewport_height:frameHeight,frame_width:frameWidth,frame_height:frameHeight}),{capability:"Visible region capture",requestVariant:"CaptureVisibleRegion",minimumProtocolVersion:443})
  const capture=response.VisibleRegionCaptured?.capture
  if(!capture || capture.capture_id!==captureId || capture.media_type!=="image/png" || capture.data_base64.length>6*1024*1024
    || !Object.entries(surface).every(([key,value])=>(capture.surface as unknown as Record<string,unknown>)[key]===value))throw new Error("Capture identity mismatch")
  const bytes=Buffer.from(capture.data_base64,"base64")
  if(bytes.length<24 || bytes.subarray(0,8).toString("hex")!=="89504e470d0a1a0a" || bytes.readUInt32BE(16)!==capture.width || bytes.readUInt32BE(20)!==capture.height || capture.width!==width || capture.height!==height || createHash("sha256").update(bytes).digest("hex")!==capture.sha256)throw new Error("Capture digest mismatch")
  input.check()
  const root=await mkdtemp(path.join(os.tmpdir(),"chariox-region-capture-"))
  try {
    const filename=`chariox-${kind}-${id.replace(/[^a-zA-Z0-9-]/g,"-").slice(0,64)}-${capture.captured_at_ms}.png`
    const file=path.join(root,filename)
    await writeFile(file,bytes,{mode:0o600})
    input.check()
    await input.attach({path:file,filename,mime:"image/png",kind:"image"})
  } finally { await rm(root,{recursive:true,force:true}) }
}
