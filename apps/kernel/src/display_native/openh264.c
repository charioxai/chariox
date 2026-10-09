/* MP-08/MP-10 (owner 2026-10-09, as Selkies): OpenH264 is the default native
 * encoder, loaded at runtime (Cisco's binary), never linked into the kernel.
 * Kept apart from codec.c: x264.h and the OpenH264 API share NAL_* names. */
#include <dlfcn.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "openh264/codec_api.h"
#include "openh264/codec_ver.h"

static int (*create_encoder)(ISVCEncoder **);
static void (*destroy_encoder)(ISVCEncoder *);
static void (*codec_version)(OpenH264Version *);
static pthread_once_t once=PTHREAD_ONCE_INIT;
static int ready;
static void load(void) {
    /* The vendored 2.6 headers are ABI 8 (libopenh264-2.6.x-linux64.8.so); an
     * absolute CHARIOX_BROWSER_DISPLAY_OPENH264 names that binary directly. */
    const char *path=getenv("CHARIOX_BROWSER_DISPLAY_OPENH264");
    void *library=dlopen(path&&path[0]=='/'?path:"libopenh264.so.8",RTLD_NOW|RTLD_LOCAL);
    if (!library) return;
    *(void **)&create_encoder=dlsym(library,"WelsCreateSVCEncoder");
    *(void **)&destroy_encoder=dlsym(library,"WelsDestroySVCEncoder");
    *(void **)&codec_version=dlsym(library,"WelsGetCodecVersionEx");
    if (!create_encoder||!destroy_encoder||!codec_version) return;
    OpenH264Version version={0};codec_version(&version);
    ready=version.uMajor==OPENH264_MAJOR&&version.uMinor==OPENH264_MINOR;
}
int cx_openh264_available(void) {pthread_once(&once,load);return ready;}

void cx_openh264_close(void *encoder) {
    ISVCEncoder *e=encoder;
    if (e) {(*e)->Uninitialize(e);destroy_encoder(e);}
}
/* The x264 stream contract: constrained baseline, Annex B with parameter sets
 * on every IDR, infinite GOP (IDRs only on reset), no skipped frames. */
void *cx_openh264_open(int width,int height,int bitrate,int threads,int max_qp) {
    ISVCEncoder *e=NULL;SEncParamExt p;
    if (!cx_openh264_available()||create_encoder(&e)||!e) return NULL;
    int quiet=WELS_LOG_QUIET; /* The worker's stderr carries no codec traces. */
    if ((*e)->SetOption(e,ENCODER_OPTION_TRACE_LEVEL,&quiet)||(*e)->GetDefaultParams(e,&p)) {cx_openh264_close(e);return NULL;}
    p.iUsageType=SCREEN_CONTENT_REAL_TIME;p.iPicWidth=width;p.iPicHeight=height;p.iTargetBitrate=bitrate;p.iRCMode=RC_BITRATE_MODE;p.fMaxFrameRate=60;
    p.iTemporalLayerNum=1;p.iSpatialLayerNum=1;p.uiIntraPeriod=0;p.iNumRefFrame=1;p.eSpsPpsIdStrategy=CONSTANT_ID;
    p.bPrefixNalAddingCtrl=false;p.bEnableSSEI=false;p.iEntropyCodingModeFlag=0;p.iPaddingFlag=0;
    p.bEnableFrameSkip=false;p.iMaxBitrate=UNSPECIFIED_BIT_RATE;p.bEnableSceneChangeDetect=false;p.bEnableLongTermReference=false;
    p.bEnableDenoise=false;p.iMultipleThreadIdc=(unsigned short)threads;
    /* MP-08/MP-10/MP-11: a protected native row already caps QP at36
     * for independently checked black fields. OpenH264's initial screen key
     * starts below that cap and can exceed the1MiB packet bound before rate
     * control has any history. Start at the same protected QP cap; all output
     * checks, native geometry and exact repair remain unchanged. */
    if (max_qp>0) p.iMinQp=p.iMaxQp=max_qp;
    SSpatialLayerConfig *layer=&p.sSpatialLayers[0];
    layer->iVideoWidth=width;layer->iVideoHeight=height;layer->fFrameRate=60;layer->iSpatialBitrate=bitrate;layer->iMaxSpatialBitrate=UNSPECIFIED_BIT_RATE;
    layer->uiProfileIdc=PRO_BASELINE;layer->uiLevelIdc=LEVEL_5_1;layer->bFullRange=false;
    layer->sSliceArgument.uiSliceMode=threads>1?SM_FIXEDSLCNUM_SLICE:SM_SINGLE_SLICE;layer->sSliceArgument.uiSliceNum=threads>1?(unsigned)threads:1;
    int format=videoFormatI420;
    if ((*e)->InitializeExt(e,&p)||(*e)->SetOption(e,ENCODER_OPTION_DATAFORMAT,&format)) {cx_openh264_close(e);return NULL;}
    return e;
}
int cx_openh264_rate(void *encoder,int bitrate) {
    ISVCEncoder *e=encoder;SBitrateInfo rate={SPATIAL_LAYER_ALL,bitrate};
    return (*e)->SetOption(e,ENCODER_OPTION_BITRATE,&rate)?-1:0;
}
/* One access unit (every layer) into *packet; returns its length, or -1. */
int cx_openh264_encode(void *encoder,uint8_t *const planes[3],const int strides[3],int width,int height,uint64_t sequence,uint8_t **packet,size_t *capacity,int *key) {
    ISVCEncoder *e=encoder;SSourcePicture picture;SFrameBSInfo info;
    memset(&picture,0,sizeof(picture));memset(&info,0,sizeof(info));
    picture.iColorFormat=videoFormatI420;picture.iPicWidth=width;picture.iPicHeight=height;
    for (int i=0;i<3;i++){picture.iStride[i]=strides[i];picture.pData[i]=planes[i];}
    picture.uiTimeStamp=(long long)(sequence*1000/60);
    if ((*e)->EncodeFrame(e,&picture,&info)!=cmResultSuccess) return -1;
    if (info.eFrameType==videoFrameTypeSkip||info.eFrameType==videoFrameTypeInvalid||info.iFrameSizeInBytes<=0||info.iFrameSizeInBytes>1024*1024) return -1;
    if (*capacity<(size_t)info.iFrameSizeInBytes) {
        uint8_t *buffer=realloc(*packet,info.iFrameSizeInBytes);
        if (!buffer) return -1;
        *packet=buffer;*capacity=info.iFrameSizeInBytes;
    }
    size_t offset=0;
    for (int l=0;l<info.iLayerNum;l++) {
        const SLayerBSInfo *layer=&info.sLayerInfo[l];size_t size=0;
        for (int n=0;n<layer->iNalCount;n++) size+=layer->pNalLengthInByte[n];
        if (offset+size>(size_t)info.iFrameSizeInBytes) return -1;
        memcpy(*packet+offset,layer->pBsBuf,size);offset+=size;
    }
    *key=info.eFrameType==videoFrameTypeIDR;
    return offset==(size_t)info.iFrameSizeInBytes?(int)offset:-1;
}
