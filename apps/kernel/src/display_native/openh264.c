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
 * macroblocks and a bits-per-activity model learned from encoded frames.
 * An IDR (a new document or geometry) may spend CX_IDR_FRAMES of the rate: at
 * 1.5 frames a page IDR sits at QP 51 and cannot be read; the frames after it
 * repay the VBV before they refine. */
struct Encoder {ISVCEncoder *e;SEncParamExt p;uint8_t *previous;double fill,*model;int qp,max_qp;};
void cx_openh264_close(void *encoder) {
    struct Encoder *h=encoder;
    if (h) {(*h->e)->Uninitialize(h->e);destroy_encoder(h->e);free(h->previous);free(h);}
}
#define CX_QP_MIN 24
#define CX_QP_MAX 51
#define CX_IDR_FRAMES 5
/* A refinement frame (nothing changed) lowers the QP this much while the VBV has room. */
#define CX_REFINE_STEP 4
static double frame_bits(const struct Encoder *h) {return h->p.iTargetBitrate/60.;}
#if defined(__SSE2__)
#include <emmintrin.h>
#endif
/* Sum of |neighbour differences| of one 16x16 luma block: 15 horizontal and 15 vertical steps per row/column pair. */
static int block_gradient(const uint8_t *p,int stride) {
    int sum=64;
#if defined(__SSE2__)
    /* PSADBW sums eight byte differences at once. Lane 15 is masked off the horizontal step, so no row is read past its block. */
    const __m128i lanes=_mm_set_epi8(0,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1);
    __m128i total=_mm_setzero_si128();
    for (int r=0;r<16;r++) {
        const uint8_t *row=p+(size_t)r*stride;
        __m128i here=_mm_loadu_si128((const __m128i *)row);
        total=_mm_add_epi64(total,_mm_sad_epu8(_mm_and_si128(here,lanes),_mm_srli_si128(here,1)));
        if (r) total=_mm_add_epi64(total,_mm_sad_epu8(here,_mm_loadu_si128((const __m128i *)(row-stride))));
    }
    sum+=_mm_cvtsi128_si32(total)+_mm_cvtsi128_si32(_mm_srli_si128(total,8));
#else
    for (int r=0;r<16;r++) {
        const uint8_t *row=p+(size_t)r*stride;
        for (int x=1;x<16;x++) sum+=abs(row[x]-row[x-1]);
        if (r) for (int x=0;x<16;x++) sum+=abs(row[x]-row[x-stride]);
    }
#endif
    return sum;
}
/* Gradient activity of the macroblocks that differ from the previous input (all of them without
 * `compare`). One pass: a changed block's luma is also copied into `previous`, still in cache. */
static double activity(const uint8_t *y,int stride,uint8_t *previous,int compare,int width,int height,int *changed) {
    double total=0;*changed=0;
    for (int my=0;my<height/16;my++) for (int mx=0;mx<width/16;mx++) {
        const uint8_t *p=y+(size_t)(my*16)*stride+mx*16;uint8_t *q=previous+(size_t)(my*16)*width+mx*16;
        int differs=!compare;
        for (int r=0;r<16&&!differs;r++) differs=memcmp(p+(size_t)r*stride,q+(size_t)r*width,16)!=0;
        if (!differs) continue;
        total+=block_gradient(p,stride);(*changed)++;
        for (int r=0;r<16;r++) memcpy(q+(size_t)r*width,p+(size_t)r*stride,16);
    }
    return total;
}
/* Exposed for the exactness test only. */
double cx_openh264_activity(const uint8_t *y,int stride,uint8_t *previous,int compare,int width,int height,int *changed) {return activity(y,stride,previous,compare,width,height,changed);}
/* Size roughly halves every 8 QP; the model is bits per activity at QP 45. */
static double scale(int qp) {return exp2((45-qp)/8.);}
/* The x264 stream contract: constrained baseline, Annex B with parameter sets
 * on every IDR, infinite GOP (IDRs only on reset), no skipped frames. Screen
 * content usage forces scene-change IDRs and costs ~4x the encode CPU of
 * camera usage (2560x1600 motion 31 vs 7 ms), which a loaded host turns
 * directly into input latency; rate control is ours either way. model
 * holds two bits-per-activity estimates (intra, inter) that outlive the
 * encoder, so a reset's IDR starts from what earlier IDRs cost. */
/* Sliced threads spin: on a quiet 16-vCPU host four threads cost 10 ms of CPU per 1264x708 frame (17 ms at
 * 1920x1080) against 3.3 ms (5.8 ms) for one, with the same or a longer wall time; two threads win only
 * from about 3 MP (2528x1416: 10.8 ms wall, 18 ms CPU against 16.7 ms for one). `limit` is the caller's cap. */
int cx_openh264_slice_threads(int width,int height,int limit) {
    int want=(long)width*height>2500000?2:1;
    return limit<1?1:want<limit?want:limit;
}
void *cx_openh264_open(int width,int height,int bitrate,int threads,int max_qp,double *model) {
    ISVCEncoder *e=NULL;struct Encoder *h;
    threads=cx_openh264_slice_threads(width,height,threads);
    if (!cx_openh264_available()||create_encoder(&e)||!e) return NULL;
    if (!(h=calloc(1,sizeof(*h)))) {destroy_encoder(e);return NULL;}
    h->e=e;h->model=model;h->qp=h->max_qp=max_qp>0?max_qp:CX_QP_MAX;
    SEncParamExt *p=&h->p;
    int quiet=WELS_LOG_QUIET; /* The worker's stderr carries no codec traces. */
    if ((*e)->SetOption(e,ENCODER_OPTION_TRACE_LEVEL,&quiet)||(*e)->GetDefaultParams(e,p)) {cx_openh264_close(h);return NULL;}
    p->iUsageType=CAMERA_VIDEO_REAL_TIME;p->iPicWidth=width;p->iPicHeight=height;p->iTargetBitrate=bitrate;p->iRCMode=RC_OFF_MODE;p->fMaxFrameRate=60;
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
/* The largest IDR this encoder means to send: its budget plus the model's error. */
int cx_openh264_key_bytes(void *encoder) {return (int)(1.25*CX_IDR_FRAMES*frame_bits(encoder)/8);}
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
    double busy=activity(planes[0],strides[0],h->previous,sequence!=0,width,height,&changed);
    /* A mostly changed frame costs about an intra frame. */
    int intra=!sequence||changed*5>=blocks*4;
    double rate=h->model[!intra];
    h->fill=sequence?fmin(1.5*frame_bits(h),h->fill+frame_bits(h)):CX_IDR_FRAMES*frame_bits(h);
    int qp;
    if (!changed) {
        /* Refinement of an unchanged picture: its cost is the coarse reference, not its activity, so it
         * steps down only while the VBV has room and otherwise holds the QP (a nearly empty frame). */
        qp=h->fill>0?h->qp-CX_REFINE_STEP:h->qp;
        if (qp<CX_QP_MIN) qp=CX_QP_MIN;
    } else {
        /* Refining a coarse reference costs more than its activity: an inter
         * frame's QP falls by one per frame. */
        qp=intra?CX_QP_MIN:h->qp-1<CX_QP_MIN?CX_QP_MIN:h->qp-1;
        while (qp<h->max_qp&&rate*busy*scale(qp)>h->fill) qp++;
    }
    if (qp>h->max_qp) qp=h->max_qp;
    if (qp!=h->qp) {
        h->p.sSpatialLayers[0].iDLayerQp=qp;
        if ((*e)->SetOption(e,ENCODER_OPTION_SVC_ENCODE_PARAM_EXT,&h->p)) return -1;
        h->qp=qp;
    }
    if ((*e)->EncodeFrame(e,&picture,&info)!=cmResultSuccess) return -1;
    double bits=8.*info.iFrameSizeInBytes;
    /* A refinement repays all it spent: the descent then paces itself at the negotiated rate. */
    h->fill=changed?fmax(h->fill-bits,-1.5*frame_bits(h)):h->fill-bits;
    if (busy>0&&bits>0) {double *m=&h->model[!intra];*m=(*m+bits/(busy*scale(qp)))/2;}
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
