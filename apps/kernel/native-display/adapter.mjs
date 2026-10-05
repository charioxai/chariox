// MD-DISPLAY-04: design stubs only; not registered with protocol 419.
// Capture must pass the kernel document/Vault barrier before encodeProtected.
export function plannedNativeAdapter(platform) {
 const plan = {
  darwin: {capture:'protected CDP PNG first; ScreenCaptureKit window later',encoder:'VideoToolbox VTCompressionSession',input:'protected sRGB CVPixelBuffer',codec:'H.264, pending negotiated presenter support'},
  win32: {capture:'protected CDP PNG first; Windows.Graphics.Capture window later',encoder:'Media Foundation IMFTransform',input:'protected sRGB BGRA -> NV12 sample',codec:'H.264, pending negotiated presenter support'},
 }[platform];
 if(!plan)throw Error('MD-DISPLAY: unknown native platform');
 const unavailable=async()=>{throw Error('MD-DISPLAY: native display adapter is a design stub; use portable VP9/PNG')};
 return {...plan,available:false,captureProtected:unavailable,encodeProtected:unavailable,async close(){}};
}
