/* MP-08/MP-10 (owner 2026-10-09, as Selkies): OpenH264 is the default native
 * encoder, loaded at runtime (Cisco's binary), never linked into the kernel.
 * Kept apart from codec.c: x264.h and the OpenH264 API share NAL_* names. */
#include <dlfcn.h>
#include <math.h>
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

/* MP-08/MP-10: OpenH264's bitrate RC starts an IDR at a fixed QP from its
 * bits-per-pixel table and moves a P frame's QP a few steps at a time, so a
 * full-page change costs 4-8x x264's VBV-bounded frame and every byte delays
 * the visible echo. Like x264 (row_vbv), each frame here must fit a VBV of
 * 1.5 frames at the negotiated rate: RC is off and the frame QP is the lowest
 * whose predicted size fits, from the luma activity of the changed
 * macroblocks and a bits-per-activity model learned from encoded frames. */
struct Encoder {ISVCEncoder *e;SEncParamExt p;uint8_t *previous;double fill,*model;int qp,max_qp;};
void cx_openh264_close(void *encoder) {
    struct Encoder *h=encoder;
    if (h) {(*h->e)->Uninitialize(h->e);destroy_encoder(h->e);free(h->previous);free(h);}
}
#define CX_QP_MIN 24
#define CX_QP_MAX 51
static double frame_bits(const struct Encoder *h) {return h->p.iTargetBitrate/60.;}
/* Gradient activity of the macroblocks that differ from the previous input. */
static double activity(const uint8_t *y,int stride,const uint8_t *previous,int width,int height,int *changed) {
    double total=0;*changed=0;
    for (int my=0;my<height/16;my++) for (int mx=0;mx<width/16;mx++) {
        int differs=!previous,sum=64;
        for (int r=0;r<16;r++) {
            const uint8_t *p=y+(size_t)(my*16+r)*stride+mx*16;
            if (!differs&&memcmp(p,previous+(size_t)(my*16+r)*width+mx*16,16)) differs=1;
            for (int x=1;x<16;x++) sum+=abs(p[x]-p[x-1]);
            if (r) for (int x=0;x<16;x++) sum+=abs(p[x]-p[x-stride]);
        }
        if (differs) {total+=sum;(*changed)++;}
    }
    return total;
}
/* Size roughly halves every 8 QP; the model is bits per activity at QP 45. */
static double scale(int qp) {return exp2((45-qp)/8.);}
/* The x264 stream contract: constrained baseline, Annex B with parameter sets
 * on every IDR, infinite GOP (IDRs only on reset), no skipped frames. model
 * holds two bits-per-activity estimates (intra, inter) that outlive the
 * encoder, so a reset's IDR starts from what earlier IDRs cost. */
void *cx_openh264_open(int width,int height,int bitrate,int threads,int max_qp,double *model) {
    ISVCEncoder *e=NULL;struct Encoder *h;
    if (!cx_openh264_available()||create_encoder(&e)||!e) return NULL;
    if (!(h=calloc(1,sizeof(*h)))) {destroy_encoder(e);return NULL;}
    h->e=e;h->model=model;h->qp=h->max_qp=max_qp>0?max_qp:CX_QP_MAX;
    SEncParamExt *p=&h->p;
    int quiet=WELS_LOG_QUIET; /* The worker's stderr carries no codec traces. */
    if ((*e)->SetOption(e,ENCODER_OPTION_TRACE_LEVEL,&quiet)||(*e)->GetDefaultParams(e,p)) {cx_openh264_close(h);return NULL;}
    p->iUsageType=SCREEN_CONTENT_REAL_TIME;p->iPicWidth=width;p->iPicHeight=height;p->iTargetBitrate=bitrate;p->iRCMode=RC_OFF_MODE;p->fMaxFrameRate=60;
    p->iTemporalLayerNum=1;p->iSpatialLayerNum=1;p->uiIntraPeriod=0;p->iNumRefFrame=1;p->eSpsPpsIdStrategy=CONSTANT_ID;
    p->bPrefixNalAddingCtrl=false;p->bEnableSSEI=false;p->iEntropyCodingModeFlag=0;p->iPaddingFlag=0;
    p->bEnableFrameSkip=false;p->iMaxBitrate=UNSPECIFIED_BIT_RATE;p->bEnableSceneChangeDetect=false;p->bEnableLongTermReference=false;
    p->bEnableDenoise=false;p->iMultipleThreadIdc=(unsigned short)threads;
    SSpatialLayerConfig *layer=&p->sSpatialLayers[0];
    layer->iVideoWidth=width;layer->iVideoHeight=height;layer->fFrameRate=60;layer->iSpatialBitrate=bitrate;layer->iMaxSpatialBitrate=UNSPECIFIED_BIT_RATE;
    layer->uiProfileIdc=PRO_BASELINE;layer->uiLevelIdc=LEVEL_5_1;layer->bFullRange=false;layer->iDLayerQp=h->qp;
    layer->sSliceArgument.uiSliceMode=threads>1?SM_FIXEDSLCNUM_SLICE:SM_SINGLE_SLICE;layer->sSliceArgument.uiSliceNum=threads>1?(unsigned)threads:1;
    int format=videoFormatI420;
    if ((*e)->InitializeExt(e,p)||(*e)->SetOption(e,ENCODER_OPTION_DATAFORMAT,&format)||!(h->previous=malloc((size_t)width*height))) {cx_openh264_close(h);return NULL;}
    h->fill=1.5*frame_bits(h);
    return h;
}
int cx_openh264_rate(void *encoder,int bitrate) {
    struct Encoder *h=encoder;
    h->p.iTargetBitrate=h->p.sSpatialLayers[0].iSpatialBitrate=bitrate;
    return 0;
}
/* The next frame is an IDR with parameter sets; the RC state is kept. */
int cx_openh264_restart(void *encoder) {
    struct Encoder *h=encoder;
    return (*h->e)->ForceIntraFrame(h->e,true)?-1:0;
}
/* One access unit (every layer) into *packet; returns its length, or -1. */
int cx_openh264_encode(void *encoder,uint8_t *const planes[3],const int strides[3],int width,int height,uint64_t sequence,uint8_t **packet,size_t *capacity,int *key) {
    struct Encoder *h=encoder;ISVCEncoder *e=h->e;SSourcePicture picture;SFrameBSInfo info;
    memset(&picture,0,sizeof(picture));memset(&info,0,sizeof(info));
    picture.iColorFormat=videoFormatI420;picture.iPicWidth=width;picture.iPicHeight=height;
    for (int i=0;i<3;i++){picture.iStride[i]=strides[i];picture.pData[i]=planes[i];}
    picture.uiTimeStamp=(long long)(sequence*1000/60);
    int changed,blocks=(width/16)*(height/16);
    double busy=activity(planes[0],strides[0],sequence?h->previous:NULL,width,height,&changed);
    /* A mostly changed frame is coded intra (screen content forces scene-change IDRs). */
    int intra=!sequence||changed*5>=blocks*4;
    double rate=h->model[!intra];
    h->fill=fmin(1.5*frame_bits(h),h->fill+frame_bits(h));
    /* Refining a coarse reference costs more than its activity: an inter
     * frame's QP falls by one per frame. */
    int qp=intra?CX_QP_MIN:h->qp-1<CX_QP_MIN?CX_QP_MIN:h->qp-1;
    while (qp<h->max_qp&&rate*busy*scale(qp)>h->fill) qp++;
    if (qp>h->max_qp) qp=h->max_qp;
    if (qp!=h->qp) {
        h->p.sSpatialLayers[0].iDLayerQp=qp;
        if ((*e)->SetOption(e,ENCODER_OPTION_SVC_ENCODE_PARAM_EXT,&h->p)) return -1;
        h->qp=qp;
    }
    if ((*e)->EncodeFrame(e,&picture,&info)!=cmResultSuccess) return -1;
    for (int y=0;y<height;y++) memcpy(h->previous+(size_t)y*width,planes[0]+(size_t)y*strides[0],width);
    double bits=8.*info.iFrameSizeInBytes;
    h->fill=fmax(h->fill-bits,-1.5*frame_bits(h));
    if (busy>0&&bits>0) {double *m=&h->model[info.eFrameType!=videoFrameTypeIDR];*m=(*m+bits/(busy*scale(qp)))/2;}
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
