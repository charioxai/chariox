// MP-08/MP-10/MP-11: original bounded OpenH264 screen-content adapter.
// No capture or I/O. Build against Cisco API headers; load only in Python helper.
#include <wels/codec_api.h>
#include <cstdint>
#include <cstring>
#include <new>
struct Context { ISVCEncoder* encoder; int width, height; };
extern "C" int chariox_h264_version() {
  const auto v = WelsGetCodecVersion();
  return static_cast<int>(v.uMajor * 10000 + v.uMinor * 100 + v.uRevision);
}
extern "C" void chariox_h264_close(void* handle) {
  auto* c = static_cast<Context*>(handle);
  if (!c) return;
  c->encoder->Uninitialize(); WelsDestroySVCEncoder(c->encoder); delete c;
}
extern "C" void* chariox_h264_create(int width, int height, int bitrate) {
  if (width < 16 || width > 2560 || height < 16 || height > 1600 ||
      width % 2 || height % 2 || bitrate < 16000 || bitrate > 64000000) return nullptr;
  ISVCEncoder* encoder = nullptr;
  if (WelsCreateSVCEncoder(&encoder) != 0 || !encoder) return nullptr;
  int silent = 0; encoder->SetOption(ENCODER_OPTION_TRACE_LEVEL, &silent);
  SEncParamExt p{};
  if (encoder->GetDefaultParams(&p) != 0) { WelsDestroySVCEncoder(encoder); return nullptr; }
  p.iUsageType = SCREEN_CONTENT_REAL_TIME;
  p.iPicWidth = width; p.iPicHeight = height;
  p.iTargetBitrate = bitrate; p.iMaxBitrate = bitrate;
  p.iRCMode = RC_BITRATE_MODE; p.fMaxFrameRate = 60;
  p.iSpatialLayerNum = 1; p.iTemporalLayerNum = 1;
  p.iMultipleThreadIdc = 1; p.bEnableFrameSkip = false;
  p.uiIntraPeriod = 120; p.iComplexityMode = LOW_COMPLEXITY;
  auto& layer = p.sSpatialLayers[0];
  layer.iVideoWidth = width; layer.iVideoHeight = height;
  layer.fFrameRate = 60; layer.iSpatialBitrate = bitrate; layer.iMaxSpatialBitrate = bitrate;
  layer.uiProfileIdc = PRO_BASELINE; layer.uiLevelIdc = LEVEL_5_1;
  layer.sSliceArgument.uiSliceMode = SM_SINGLE_SLICE;
  if (encoder->InitializeExt(&p) != 0) { WelsDestroySVCEncoder(encoder); return nullptr; }
  SEncParamExt actual{};
  if (encoder->GetOption(ENCODER_OPTION_SVC_ENCODE_PARAM_EXT, &actual) != 0 || actual.iUsageType != SCREEN_CONTENT_REAL_TIME) {
    encoder->Uninitialize(); WelsDestroySVCEncoder(encoder); return nullptr;
  }
  auto* c = new(std::nothrow) Context{encoder, width, height};
  if (!c) { encoder->Uninitialize(); WelsDestroySVCEncoder(encoder); }
  return c;
}
extern "C" int chariox_h264_encode(void* handle, uint8_t* y, uint8_t* u, uint8_t* v,
    int stride_y, int stride_u, int stride_v, int64_t timestamp, int force_key,
    uint8_t* output, int capacity) {
  auto* c = static_cast<Context*>(handle);
  if (!c || !y || !u || !v || !output || capacity < 1 || capacity > 1024*1024 ||
      stride_y < c->width || stride_u < c->width/2 || stride_v < c->width/2) return -1;
  if (force_key && c->encoder->ForceIntraFrame(true) != 0) return -1;
  SSourcePicture picture{};
  picture.iPicWidth = c->width; picture.iPicHeight = c->height;
  picture.iColorFormat = videoFormatI420; picture.uiTimeStamp = timestamp;
  picture.pData[0] = y; picture.pData[1] = u; picture.pData[2] = v;
  picture.iStride[0] = stride_y; picture.iStride[1] = stride_u; picture.iStride[2] = stride_v;
  SFrameBSInfo info{};
  if (c->encoder->EncodeFrame(&picture, &info) != 0 || info.iLayerNum < 1 || info.iLayerNum > MAX_LAYER_NUM_OF_FRAME) return -1;
  int size = 0;
  for (int i = 0; i < info.iLayerNum; ++i) {
    const auto& layer = info.sLayerInfo[i];
    if (!layer.pBsBuf || !layer.pNalLengthInByte || layer.iNalCount < 1 || layer.iNalCount > 256) return -1;
    int bytes = 0;
    for (int n = 0; n < layer.iNalCount; ++n) {
      const int length = layer.pNalLengthInByte[n];
      if (length < 1 || length > capacity - size - bytes) return -1;
      bytes += length;
    }
    std::memcpy(output + size, layer.pBsBuf, bytes); size += bytes;
  }
  return size;
}
