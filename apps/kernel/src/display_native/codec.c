/* MP-08/MP-10/MP-11: native software stripes with identical input/output
 * masking gates. Codec dependencies remain private until admitted delivery. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <stdio.h>
static double cpu_ms(void) {struct timespec t;clock_gettime(CLOCK_THREAD_CPUTIME_ID,&t);return t.tv_sec*1000.+t.tv_nsec/1000000.;}
#include <x264.h>
#include <libyuv/convert.h>
#include <libyuv/convert_from.h>
#include <libyuv/scale_argb.h>
#include <libavcodec/avcodec.h>
#include <libavutil/frame.h>
#include <libavutil/hwcontext.h>
#include <libavutil/opt.h>
#include <libavutil/log.h>
#include <libavutil/error.h>
#include <stdarg.h>
#include <pthread.h>

struct Rect { int left,top,right,bottom; };
struct RowResult { int row,y,height,key; uint64_t sequence,reference; const uint8_t *bytes; size_t length; };
struct Row {
    x264_t *codec;
    x264_picture_t picture;
    int allocated,width,height;
    AVCodecContext *hardware;
    AVFrame *staging;
    AVCodecContext *decoder;
    AVFrame *decoded;
    uint8_t *previous,*packet,*rgb;
    float *offsets; /* MP-11: per-macroblock x264 quant offsets of protected rows. */
    size_t packet_capacity;
    uint64_t sequence;
    /* x264 reconstruction (NV12) of the last encoded frame: bit-exact with
     * normative H.264 decoding; valid until this row's next encode/close. */
    const uint8_t *recon_y,*recon_uv;
    int recon_y_stride,recon_uv_stride;
};
struct Codec { int width,height,bitrate,row_count,reduced,enc_width,enc_height; struct Row rows[8]; uint8_t *masked,*full; double cpu[6]; AVBufferRef *device; int hardware_requested,fallback; char diagnostic[4096]; };
/* MP-10/MP-11: bounded driver-only logs; never page content or pixels. */
static _Thread_local struct Codec *diagnosing;
static pthread_once_t diagnostic_once=PTHREAD_ONCE_INIT;
static void diagnostic_append(struct Codec *c,const char *text) {
    size_t used=strlen(c->diagnostic);if(used<sizeof(c->diagnostic)-1)snprintf(c->diagnostic+used,sizeof(c->diagnostic)-used,"%s",text);
}
static void hardware_log(void *context,int level,const char *format,va_list args) {
    if(diagnosing&&level<=AV_LOG_WARNING){char text[1024];va_list copy;va_copy(copy,args);vsnprintf(text,sizeof(text),format,copy);va_end(copy);diagnostic_append(diagnosing,text);}
    av_log_default_callback(context,level,format,args);
}
static void diagnostic_install(void){av_log_set_callback(hardware_log);}
static void diagnostic_status(struct Codec *c,const char *stage,int status) {
    char error[AV_ERROR_MAX_STRING_SIZE],text[256];av_strerror(status,error,sizeof(error));snprintf(text,sizeof(text),"%s: %s (%d)\n",stage,error,status);diagnostic_append(c,text);
}
const char *cx_codec_diagnostic(struct Codec *c){return c->diagnostic;}
void cx_codec_cpu(struct Codec *c,double *out) {memcpy(out,c->cpu,sizeof(c->cpu));}
static void row_close(struct Row *row) {
    if (row->codec) x264_encoder_close(row->codec);
    if (row->allocated) x264_picture_clean(&row->picture);
    avcodec_free_context(&row->hardware);av_frame_free(&row->staging);
    avcodec_free_context(&row->decoder); av_frame_free(&row->decoded);
    free(row->previous);free(row->packet);free(row->rgb);free(row->offsets);memset(row,0,sizeof(*row));
}
void cx_codec_close(struct Codec *c) {
    if (!c) return;
    for (int r=0;r<8;r++) row_close(&c->rows[r]);
    av_buffer_unref(&c->device);free(c->masked);free(c->full);free(c);
}
struct Codec *cx_codec_open(int width,int height,int bitrate,int row_count,int reduced) {
    if (row_count!=1&&row_count!=8)return NULL;
    struct Codec *c=calloc(1,sizeof(*c));
    if (!c) return NULL;
    c->hardware_requested=!getenv("CHARIOX_BROWSER_DISPLAY_SOFTWARE")||strcmp(getenv("CHARIOX_BROWSER_DISPLAY_SOFTWARE"),"1");
    if(c->hardware_requested) {
        pthread_once(&diagnostic_once,diagnostic_install);
        for(int n=128;n<144&&!c->device;n++){
            char path[64];snprintf(path,sizeof(path),"/dev/dri/renderD%d",n);
            if(access(path,R_OK|W_OK)!=0)continue;
            diagnostic_append(c,path);diagnostic_append(c,": VAAPI device init\n");diagnosing=c;
            int status=av_hwdevice_ctx_create(&c->device,AV_HWDEVICE_TYPE_VAAPI,path,NULL,0);diagnosing=NULL;
            if(status<0)diagnostic_status(c,"av_hwdevice_ctx_create",status);
        }
        if(!c->device&&!c->diagnostic[0])diagnostic_append(c,"No accessible VAAPI render device /dev/dri/renderD128..143\n");
        if(!c->device)c->fallback=1;
    }
    c->width=c->enc_width=width;c->height=c->enc_height=height;c->bitrate=bitrate;c->row_count=row_count;c->reduced=reduced!=0;
    c->masked=malloc((size_t)width*height*4);
    if (!c->masked) { cx_codec_close(c);return NULL; }
    return c;
}
/* MP-08/MP-10: hardware owns the same masked input/output path. A failed
 * device/context is visible in diagnostics, never presented as acceleration. */
/* MP-08/MP-10: telemetry for the contention fallback actually applied. */
int cx_codec_reduced(struct Codec *c) {return c->row_count==1&&c->enc_width!=c->width;}
int cx_codec_backend(struct Codec *c) {return c->hardware_requested?(c->device&&!c->fallback?1:2):0;}
static int hardware_open(struct Codec *c,struct Row *row,int h,int rate) {
    if(!c->device||c->fallback)return -1;
    const AVCodec *encoder=avcodec_find_encoder_by_name("h264_vaapi");
    row->hardware=avcodec_alloc_context3(encoder);
    if(!encoder||!row->hardware)return -1;
    AVBufferRef *frames=av_hwframe_ctx_alloc(c->device);if(!frames)return -1;
    AVHWFramesContext *pool=(AVHWFramesContext*)frames->data;
    pool->format=AV_PIX_FMT_VAAPI;pool->sw_format=AV_PIX_FMT_NV12;pool->width=(c->width+15)&~15;pool->height=(h+15)&~15;pool->initial_pool_size=4;
    int pool_status=av_hwframe_ctx_init(frames);if(pool_status<0){av_buffer_unref(&frames);return pool_status;}
    AVCodecContext *ctx=row->hardware;
    ctx->hw_frames_ctx=frames;ctx->width=c->width;ctx->height=h;ctx->pix_fmt=AV_PIX_FMT_VAAPI;ctx->time_base=(AVRational){1,60};ctx->framerate=(AVRational){60,1};ctx->gop_size=120;ctx->max_b_frames=0;
    ctx->profile=AV_PROFILE_H264_CONSTRAINED_BASELINE;ctx->level=51;ctx->bit_rate=(int64_t)rate*1000;ctx->rc_max_rate=ctx->bit_rate;ctx->rc_buffer_size=rate*50;ctx->thread_count=1;ctx->color_range=AVCOL_RANGE_MPEG;
    AVDictionary *options=NULL;av_dict_set(&options,"rc_mode","VBR",0);av_dict_set(&options,"async_depth","1",0);
    int status=avcodec_open2(ctx,encoder,&options);av_dict_free(&options);if(status<0)return status;
    row->staging=av_frame_alloc();if(!row->staging)return -1;
    row->staging->format=AV_PIX_FMT_NV12;row->staging->width=c->width;row->staging->height=h;
    return av_frame_get_buffer(row->staging,32);
}
/* MP-08/MP-10/MP-11: motion is native resolution. Only a session opened as
 * the measured-contention fallback reduces unprotected whole-frame software
 * motion to a geometry the presenter admits; working hardware stays native.
 * Native exact repair restores the original raster; scaled reconstruction
 * certifies no native pixels. */
static void motion_geometry(struct Codec *c,int protected) {
    c->enc_width=c->width;c->enc_height=c->height;
    if(c->row_count!=1||protected||(c->device&&!c->fallback)||!c->reduced)return;
    if(c->width==1920&&c->height==1080){c->enc_width=1280;c->enc_height=720;}
    else if(c->width==2560&&c->height==1600){c->enc_width=1280;c->enc_height=800;}
}
/* MP-08/MP-10: Selkies/pixelflux rate control (encoders/software.rs): CBR
 * at the negotiated rate split by row height, VBV of 1.5 frames at 60 fps
 * (pixelflux vbv_bits with an infinite GOP), no filler. */
static int row_rate(struct Codec *c,int h) {int rate=(int)((double)c->bitrate*h/c->height/1000);return rate<16?16:rate;}
static int row_vbv(int rate) {int vbv=rate*3/120;return vbv<16?16:vbv;}
static void row_rate_control(x264_param_t *p,int rate) {
    p->rc.i_rc_method=X264_RC_ABR;p->rc.i_bitrate=rate;p->rc.i_vbv_max_bitrate=rate;
    p->rc.i_vbv_buffer_size=row_vbv(rate);p->rc.b_filler=0;
}
static int row_open(struct Codec *c,struct Row *row,int h,int protected) {
    int rate=row_rate(c,h);
    /* A failed h264_vaapi init is software from this first key onward. */
    if(c->device&&!c->fallback){diagnosing=c;int status=hardware_open(c,row,h,rate);diagnosing=NULL;if(status<0){diagnostic_status(c,"h264_vaapi encoder init",status);c->fallback=2;avcodec_free_context(&row->hardware);av_frame_free(&row->staging);motion_geometry(c,protected);}}
    x264_param_t p;
    if (x264_param_default_preset(&p,"ultrafast","zerolatency")) return -1;
    int ew=c->row_count==1?c->enc_width:c->width,eh=c->row_count==1?c->enc_height:h;
    if(ew!=c->width&&!c->full&&!(c->full=malloc((size_t)ew*eh*4)))return -1;
    p.i_width=ew;p.i_height=eh;p.i_csp=X264_CSP_I420;
    /* MP-08/MP-10: whole-frame motion uses pixelflux's sliced threads,
     * clamp(cores-1,1,4); each of the eight stripe rows stays one thread. */
    long cores=sysconf(_SC_NPROCESSORS_ONLN);
    p.i_threads=c->row_count==1?(int)(cores-1<1?1:cores-1>4?4:cores-1):1;p.i_lookahead_threads=1;p.b_sliced_threads=p.i_threads>1;
    p.b_full_recon=1; /* Exact certificates require the complete output raster. */
    p.i_fps_num=60;p.i_fps_den=1;p.i_timebase_num=1;p.i_timebase_den=60;
    /* Infinite GOP: IDRs only on session reset (lost/retired references). */
    p.i_keyint_max=X264_KEYINT_MAX_INFINITE;p.i_scenecut_threshold=0;p.i_bframe=0;p.b_repeat_headers=1;p.b_annexb=1;p.i_log_level=X264_LOG_NONE;
    p.vui.b_fullrange=0;p.i_level_idc=51;
    row_rate_control(&p,rate);
    /* Quant offsets need AQ; x264 disables AQ at zero strength, so a
     * negligible strength keeps the offsets and leaves rate control as is.
     * x264's VBV emergency QPs (52..69) zero coefficients, so masked
     * macroblocks decode grey whatever their offset: protected rows stay
     * within QP 51 and may overshoot the VBV instead. */
    if (protected&&!row->hardware){p.rc.i_aq_mode=X264_AQ_VARIANCE;p.rc.f_aq_strength=0.01f;p.rc.i_qp_max=51;}
    if (x264_param_apply_profile(&p,"baseline")) return -1;
    if(!row->hardware)row->codec=x264_encoder_open(&p);
    if ((!row->codec&&!row->hardware) || x264_picture_alloc(&row->picture,X264_CSP_I420,ew,eh)) return -1;
    row->allocated=1;row->width=ew;row->height=eh;
    if (protected&&!row->hardware&&!(row->offsets=calloc((size_t)((ew+15)/16)*((eh+15)/16),sizeof(float))))return -1;
    if (c->row_count>1&&!(row->previous=malloc((size_t)c->width*h*4))) return -1;
    /* MP-11: protected rows keep an INDEPENDENT decode of every packet; the
     * hardware path has no reconstruction. Software rows certify repairs
     * from x264's own reconstruction instead of decoding twice. */
    if (protected||row->hardware) {
        row->decoder=avcodec_alloc_context3(avcodec_find_decoder(AV_CODEC_ID_H264));
        row->decoded=av_frame_alloc(); row->rgb=protected?malloc((size_t)c->width*h*4):NULL;
        if (!row->decoder || !row->decoded || (protected&&!row->rgb)) return -1;
        row->decoder->thread_count=1;
        if (avcodec_open2(row->decoder,NULL,NULL)<0) return -1;
    }
    return 0;
}
/* MP-08/MP-10: retune a live session's rate without an IDR (pixelflux
 * reconfigure_rate). Hardware rows cannot; the caller reopens them. */
int cx_codec_rate(struct Codec *c,int bitrate) {
    for (int r=0;r<c->row_count;r++) if (c->rows[r].hardware) return -1;
    c->bitrate=bitrate;
    for (int r=0;r<c->row_count;r++) {
        struct Row *row=&c->rows[r];if(!row->codec)continue;
        int h=2*((c->height/2*(r+1))/c->row_count)-2*((c->height/2*r)/c->row_count);
        x264_param_t p;x264_encoder_parameters(row->codec,&p);row_rate_control(&p,row_rate(c,h));
        if (x264_encoder_reconfig(row->codec,&p)<0) return -1;
    }
    return 0;
}
/* All coordinates have been validated/intersected by the Rust control module.
 * Pad four pixels before conversion, then independently verify decoded RGB. */
void cx_mask(uint8_t *pixels,int width,int height,const struct Rect *regions,size_t count) {
    for (size_t n=0;n<count;n++) {
        int l=regions[n].left-4,t=regions[n].top-4,r=regions[n].right+4,b=regions[n].bottom+4;
        if (l<0)l=0;
        if (t<0)t=0;
        if (r>width)r=width;
        if (b>height)b=height;
        for (int y=t;y<b;y++) for (int x=l;x<r;x++) {
            uint8_t *p=pixels+((size_t)y*width+x)*4;
            p[0]=p[1]=p[2]=0;p[3]=255;
        }
    }
}
/* MP-08/MP-11: at a low rate (a 1.5-frame VBV IDR at Retina size) a lossy
 * macroblock straddling a mask edge flattens toward its bright neighbours,
 * and even a wholly black macroblock decodes grey at QP 51: the decoded mask
 * fails output_safe and every frame drops (IDR loop). Codec input masks cover
 * whole 16x16 macroblocks of each encoded row around the 4 px pad
 * (deblocking reaches at most 3 px into them), and those macroblocks are
 * coded at a much lower QP (flat black costs few bits). Exact paths keep
 * cx_mask; output_safe still decodes and checks every packet. */
static int mask_span(const struct Codec *c,const struct Rect *region,int y,int bottom,int *box) {
    int l=(region->left-4)&~15,r=(region->right+4+15)&~15,t=region->top-4,b=region->bottom+4;
    if (l<0)l=0;
    if (r>c->width)r=c->width;
    if (t<y)t=y;
    if (b>bottom)b=bottom;
    if (t>=b||l>=r)return 0;
    t=y+((t-y)&~15);b=y+((b-y+15)&~15);
    if (b>bottom)b=bottom;
    box[0]=l;box[1]=t;box[2]=r;box[3]=b;return 1;
}
static void mask_macroblocks(struct Codec *c,uint8_t *pixels,const struct Rect *regions,size_t count) {
    for (size_t n=0;n<count;n++) for (int row=0;row<c->row_count;row++) {
        int box[4],y=2*((c->height/2*row)/c->row_count),bottom=2*((c->height/2*(row+1))/c->row_count);
        if (!mask_span(c,&regions[n],y,bottom,box))continue;
        for (int dy=box[1];dy<box[3];dy++) for (int x=box[0];x<box[2];x++) {
            uint8_t *p=pixels+((size_t)dy*c->width+x)*4;
            p[0]=p[1]=p[2]=0;p[3]=255;
        }
    }
}
static void mask_offsets(const struct Codec *c,struct Row *row,int y,int h,const struct Rect *regions,size_t count) {
    int columns=(row->width+15)/16,rows=(row->height+15)/16;
    for (int i=0;i<columns*rows;i++)row->offsets[i]=0;
    for (size_t n=0;n<count;n++) {
        int box[4];
        if (!mask_span(c,&regions[n],y,y+h,box))continue;
        for (int my=(box[1]-y)/16;my<(box[3]-y+15)/16&&my<rows;my++) for (int mx=box[0]/16;mx<(box[2]+15)/16&&mx<columns;mx++)
            row->offsets[my*columns+mx]=-30;
    }
}
static int output_safe(struct Row *row,const uint8_t *packet,size_t length,int width,int h,int y,const struct Rect *regions,size_t count) {
    if (!row->decoder)return 1;
    av_frame_unref(row->decoded);
    AVPacket *p=av_packet_alloc();
    if (!p || length>1024*1024 || av_new_packet(p,(int)length)<0) { av_packet_free(&p);return -1; }
    memcpy(p->data,packet,length);
    int status=avcodec_send_packet(row->decoder,p);av_packet_free(&p);
    if (status<0 || avcodec_receive_frame(row->decoder,row->decoded)<0) return -1;
    AVFrame *f=row->decoded;
    if (f->width!=width || f->height!=h || (f->format!=AV_PIX_FMT_YUV420P && f->format!=AV_PIX_FMT_YUVJ420P)) return -1;
    if (!row->rgb)return 1;
    if (I420ToARGB(f->data[0],f->linesize[0],f->data[1],f->linesize[1],f->data[2],f->linesize[2],row->rgb,width*4,width,h))return -1;
    for (size_t n=0;n<count;n++) {
        int top=regions[n].top>y?regions[n].top-y:0,bottom=regions[n].bottom<y+h?regions[n].bottom-y:h;
        for (int dy=top;dy<bottom;dy++) for (int x=regions[n].left;x<regions[n].right;x++) {
            uint8_t *v=row->rgb+((size_t)dy*width+x)*4;
            if (v[0]>32 || v[1]>32 || v[2]>32) { return 0; }
        }
    }
    return 1;
}
/* MP-08/MP-10: visible dimensions differ from aligned VAAPI pool storage.
 * A driver/runtime failure restarts the SAME masked stream with software IDRs. */
static AVPacket *hardware_encode(struct Codec *c,struct Row *row,int h) {
    if(av_frame_make_writable(row->staging)<0||I420ToNV12(row->picture.img.plane[0],row->picture.img.i_stride[0],row->picture.img.plane[1],row->picture.img.i_stride[1],row->picture.img.plane[2],row->picture.img.i_stride[2],row->staging->data[0],row->staging->linesize[0],row->staging->data[1],row->staging->linesize[1],c->width,h))return NULL;
    AVFrame *frame=av_frame_alloc();if(!frame)return NULL;
    int status=av_hwframe_get_buffer(row->hardware->hw_frames_ctx,frame,0);
    if(status>=0){frame->width=c->width;frame->height=h;status=av_hwframe_transfer_data(frame,row->staging,0);}
    if(status>=0){frame->pts=(int64_t)row->sequence;frame->pict_type=row->sequence?AV_PICTURE_TYPE_NONE:AV_PICTURE_TYPE_I;status=avcodec_send_frame(row->hardware,frame);}
    av_frame_free(&frame);if(status<0)return NULL;
    AVPacket *packet=av_packet_alloc();if(!packet)return NULL;
    if(avcodec_receive_packet(row->hardware,packet)<0){av_packet_free(&packet);return NULL;}return packet;
}
/* -1: unavailable, -2: unsafe lossy output (discard ALL rows), >=0: count.
 * Exact memcmp, never a hash collision, decides unchanged-row reuse. */
int cx_codec_encode(struct Codec *c,const uint8_t *source,unsigned resets,const struct Rect *regions,size_t count,struct RowResult *results) {
    memset(c->cpu,0,sizeof(c->cpu));double at=cpu_ms();
    const uint8_t *pixels=source;
    if (count) {
        memcpy(c->masked,source,(size_t)c->width*c->height*4);cx_mask(c->masked,c->width,c->height,regions,count);mask_macroblocks(c,c->masked,regions,count);pixels=c->masked;
        for (size_t n=0;n<count;n++) for (int y=regions[n].top;y<regions[n].bottom;y++) for (int x=regions[n].left;x<regions[n].right;x++) {
            const uint8_t *p=pixels+((size_t)y*c->width+x)*4;
            if (p[0] || p[1] || p[2])return -1;
        }
    }
    c->cpu[0]+=cpu_ms()-at;
    motion_geometry(c,count>0);
    int total=0;
    for (int r=0;r<c->row_count;r++) {
        int y=2*((c->height/2*r)/c->row_count),bottom=2*((c->height/2*(r+1))/c->row_count),h=bottom-y;
        const uint8_t *data=pixels+(size_t)y*c->width*4;
        size_t size=(size_t)h*c->width*4;
        struct Row *row=&c->rows[r];
        /* Whole-frame motion only receives new capture serials; the unchanged
         * shortcut (and its reference copy) serves sparse stripe rows. */
        at=cpu_ms();
        int same=c->row_count>1 && !(resets&(1u<<r)) && (row->codec||row->hardware) && row->sequence && !memcmp(row->previous,data,size);
        c->cpu[1]+=cpu_ms()-at;
        if (same)continue;
        int protected=0;
        for (size_t n=0;n<count;n++) if (regions[n].top<bottom && regions[n].bottom>y)protected=1;
        if ((resets&(1u<<r)) || (!row->codec&&!row->hardware) || row->width!=c->enc_width || row->height!=(c->row_count==1?c->enc_height:h)) { row_close(row);if(row_open(c,row,h,protected))return -1; }
        at=cpu_ms();
        uint8_t **plane=row->picture.img.plane;int *stride=row->picture.img.i_stride;
        if (c->row_count==1&&c->enc_width!=c->width) {
            if (ARGBScale(data,c->width*4,c->width,h,c->full,c->enc_width*4,c->enc_width,c->enc_height,kFilterBox)||
                ARGBToI420(c->full,c->enc_width*4,plane[0],stride[0],plane[1],stride[1],plane[2],stride[2],c->enc_width,c->enc_height))return -1;
        } else if (ARGBToI420(data,c->width*4,plane[0],stride[0],plane[1],stride[1],plane[2],stride[2],c->width,h))return -1;
        c->cpu[2]+=cpu_ms()-at;
        row->picture.i_pts=(int64_t)row->sequence;row->picture.i_type=row->sequence?X264_TYPE_AUTO:X264_TYPE_IDR;
        x264_nal_t *nals;int n; x264_picture_t out;
        at=cpu_ms();
        AVPacket *hardware_packet=NULL;int length;
        if(row->hardware) {
            diagnosing=c;hardware_packet=hardware_encode(c,row,h);diagnosing=NULL;
            if(!hardware_packet)diagnostic_append(c,"h264_vaapi encode/transfer failed; restarting masked software stream\n");
            if(!hardware_packet){for(int i=0;i<8;i++)row_close(&c->rows[i]);c->fallback=3;return cx_codec_encode(c,source,255,regions,count,results);}
            length=hardware_packet->size;
        }else {
            if (row->offsets){mask_offsets(c,row,y,h,regions,count);row->picture.prop.quant_offsets=row->offsets;row->picture.prop.quant_offsets_free=NULL;}
            length=x264_encoder_encode(row->codec,&nals,&n,&row->picture,&out);
            if(length>0&&out.img.i_csp==X264_CSP_NV12&&out.img.i_plane==2){row->recon_y=out.img.plane[0];row->recon_uv=out.img.plane[1];row->recon_y_stride=out.img.i_stride[0];row->recon_uv_stride=out.img.i_stride[1];}
            else row->recon_y=row->recon_uv=NULL;
        }
        c->cpu[3]+=cpu_ms()-at;
        if (length<=0 || length>1024*1024 || (!hardware_packet&&n<1)){av_packet_free(&hardware_packet);return -1;}
        if (row->packet_capacity<(size_t)length) {
            uint8_t *buffer=realloc(row->packet,length);
            if (!buffer){av_packet_free(&hardware_packet);return -1;}
            row->packet=buffer;row->packet_capacity=length;
        }
        size_t packet_offset=0;int key=0;
        if(hardware_packet){memcpy(row->packet,hardware_packet->data,length);packet_offset=length;key=(hardware_packet->flags&AV_PKT_FLAG_KEY)!=0;av_packet_free(&hardware_packet);}
        else for (int i=0;i<n;i++) { if(nals[i].i_payload<0 || packet_offset+nals[i].i_payload>(size_t)length)return -1;memcpy(row->packet+packet_offset,nals[i].p_payload,nals[i].i_payload);packet_offset+=nals[i].i_payload;if(nals[i].i_type==NAL_SLICE_IDR)key=1; }
        if (packet_offset!=(size_t)length)return -1;
        at=cpu_ms();
        int safe=output_safe(row,row->packet,length,c->width,h,y,regions,count);
        c->cpu[4]+=cpu_ms()-at;
        if (safe<=0) { for(int i=0;i<8;i++)row_close(&c->rows[i]);return safe==0?-2:-1; }
        uint64_t reference=row->sequence;row->sequence++;
        if (c->row_count>1) {at=cpu_ms();memcpy(row->previous,data,size);c->cpu[5]+=cpu_ms()-at;}
        results[total++]=(struct RowResult){r,y,h,key,row->sequence,reference,row->packet,(size_t)length};
    }
    return total;
}
/* MP-08/MP-10/MP-11: exact repair may omit only pixels proved already exact.
 * Colored video reconstruction is deliberately NEVER inferred from a native
 * RGB converter: desktop/GPU rounding can differ. Limited-range neutral
 * Y=16/235,U=V=128 is exact black/white for every advertised color matrix.
 * These x264 packets set full-range=false; absent range VUI has the H.264
 * limited-range default. Lossless overlays compare their original RGB snapshot byte-for-byte.
 * The caller must bind this decoder state to DELIVERED codec revision. */
void cx_codec_repair_bounds(struct Codec *c,const uint8_t *source,const uint8_t *overlay,unsigned video_rows,int x,int y,int w,int h,int *bounds) {
    int left=x+w,top=y+h,right=x,bottom=y;
    int scaled=c->row_count==1&&c->enc_width!=c->width;
    for (int dy=y;dy<y+h;dy++) {
        int r=0;while (r+1<c->row_count && dy>=2*((c->height/2*(r+1))/c->row_count))r++;
        int row_y=2*((c->height/2*r)/c->row_count),local=dy-row_y;
        int video=(video_rows&(c->row_count==1?255:(1u<<r)))!=0;
        const struct Row *row=&c->rows[r];AVFrame *f=row->decoded;
        /* Planar decode (hardware/protected) or interleaved x264 NV12 recon. */
        const uint8_t *luma=NULL,*cb=NULL,*cr=NULL;int step=1;
        if (!scaled&&row->recon_y&&row->recon_uv&&!row->hardware) {
            luma=row->recon_y+(size_t)local*row->recon_y_stride;cb=row->recon_uv+(size_t)(local/2)*row->recon_uv_stride;cr=cb+1;step=2;
        } else if (!scaled&&f&&f->data[0]&&f->format==AV_PIX_FMT_YUV420P&&(f->color_range==AVCOL_RANGE_MPEG||f->color_range==AVCOL_RANGE_UNSPECIFIED)) {
            luma=f->data[0]+(size_t)local*f->linesize[0];cb=f->data[1]+(size_t)(local/2)*f->linesize[1];cr=f->data[2]+(size_t)(local/2)*f->linesize[2];
        }
        for (int dx=x;dx<x+w;dx++) {
            size_t offset=((size_t)dy*c->width+dx)*4;
            const uint8_t *p=source+offset;int exact=0;
            if (!video&&overlay)exact=!memcmp(p,overlay+offset,3);
            else if (video&&luma&&p[0]==p[1]&&p[1]==p[2]&&(p[0]==0||p[0]==255)) {
                exact=luma[dx]==(p[0]?235:16)&&cb[dx/2*step]==128&&cr[dx/2*step]==128;
            }
            if (!exact) {if (dx<left)left=dx;if (dy<top)top=dy;if (dx+1>right)right=dx+1;if (dy+1>bottom)bottom=dy+1;}
        }
    }
    bounds[0]=left;bounds[1]=top;bounds[2]=right;bounds[3]=bottom;
}
