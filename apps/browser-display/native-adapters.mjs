// MD-DISPLAY-02/04: native attachment contract stubs, never advertised capability.
// A future native worker owns platform permissions, surfaces and codec lifetime.
// Captured surfaces MUST cross the kernel document/policy protection fence before
// any encode result can enter the existing encrypted display transport.
export const nativeAttachmentPoints=Object.freeze({
 darwin:Object.freeze({capture:'ScreenCaptureKit SCStream, exact owned Chromium window',surface:'CVPixelBuffer at native DPR, sRGB',encode:'VideoToolbox VTCompressionSession, real-time, no reordering',fallback:'document-bound CDP + portable encoder'}),
 win32:Object.freeze({capture:'Windows Graphics Capture, exact owned Chromium window',surface:'D3D11 texture at native DPR, explicit colour conversion',encode:'Media Foundation H.264 MFT, low-latency mode',fallback:'document-bound CDP + portable encoder'}),
 linux:Object.freeze({capture:'kernel-created Xvfb, owned Chromium window backing pixmap via XDamage/XShm',surface:'native-DPR BGR0, document/visibility/policy fence; DMA-BUF later',encode:'VAAPI/QSV/NVENC synthetic roundtrip probe, software otherwise',fallback:'document-bound CDP + portable encoder'}),
});
export function nativeAdapterStub(platform){
 const point=nativeAttachmentPoints[platform];if(!point)throw Error('MD-DISPLAY: unsupported platform');
 const unavailable=async()=>{throw Error('MD-DISPLAY: native capture/encode adapter unimplemented; use portable fallback')};
 return {attachment:point,supported:false,capture:unavailable,encode:unavailable,close:async()=>{}};
}
