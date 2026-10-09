/* MP-08/MP-10: audit bitrate arguments at the real runtime encoder seam. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <libyuv/convert_argb.h>
#include <libyuv/row.h>
struct Rect {int left,top,right,bottom;};
struct RowResult {int row,y,height,key;uint64_t sequence,reference;const uint8_t *bytes;size_t length;};
void *cx_codec_open(int,int,int,int,int);
void cx_codec_close(void *);
int cx_codec_openh264(void *);
int cx_codec_reduced(void *);
int cx_codec_encode(void *,const uint8_t *,unsigned,const struct Rect *,size_t,struct RowResult *);
int cx_codec_rate(void *,int);
void cx_codec_repair_bounds(void *,const uint8_t *,const uint8_t *,unsigned,int,int,int,int,int *);
void *cx_openh264_open(int,int,int,int,int);
int cx_openh264_rate(void *,int);
static int expected,expected_max_qp,opens,retunes;
void *audited_openh264_open(int width,int height,int bitrate,int threads,int max_qp) {
    printf("MP-08/MP-10 open bitrate=%d expected=%d\n",bitrate,expected);fflush(stdout);
    assert(bitrate==expected&&max_qp==expected_max_qp);opens++;
    return cx_openh264_open(width,height,bitrate,threads,max_qp);
}
int audited_openh264_rate(void *encoder,int bitrate) {
    printf("MP-08/MP-10 retune bitrate=%d expected=%d\n",bitrate,expected);fflush(stdout);
    assert(bitrate==expected);retunes++;
    return cx_openh264_rate(encoder,bitrate);
}
static void clipped_white_is_exact(void) {
    /* MP-08/MP-10/MP-11: limited-range neutral white clips identically in
     * the supported SD, HD and UHD conversion matrices. Never certify grey
     * source pixels or non-neutral chroma from this proof. */
    const struct YuvConstants *matrices[]={&kYuvI601Constants,&kYuvH709Constants,&kYuv2020Constants};
    uint8_t y[4],u=128,v=128,rgb[16];
    for (int luma=235;luma<=255;luma++) for (int m=0;m<3;m++) {
        memset(y,luma,sizeof(y));
        assert(!I420ToARGBMatrix(y,2,&u,1,&v,1,rgb,8,matrices[m],2,2));
        for (int i=0;i<16;i++) assert(rgb[i]==255);
    }
}
int main(int argc,char **argv) {
    if(argc==2) {
        void *codec=cx_codec_open(64,64,1000000,1,0);
        if(!strcmp(argv[1],"missing")) {
            assert(!codec);puts("MP-08/MP-10 missing OpenH264 does not opt into x264 PASS");
        } else {
            assert(!strcmp(argv[1],"x264")&&codec&&!cx_codec_openh264(codec));
            uint8_t pixels[64*64*4];memset(pixels,255,sizeof(pixels));struct RowResult result[8];
            assert(cx_codec_encode(codec,pixels,255,NULL,0,result)==1);cx_codec_close(codec);
            puts("MP-08/MP-10 explicit runtime x264 opt-in PASS");
        }
        return 0;
    }
    clipped_white_is_exact();
    const int width=1280,height=800;unsigned char *pixels=malloc(width*height*4);
    memset(pixels,255,width*height*4);
    for(int rows=1;rows<=8;rows+=7) {
        struct RowResult result[8];expected=8000000/rows;
        void *codec=cx_codec_open(width,height,8000000,rows,0);assert(codec&&cx_codec_openh264(codec));
        assert(cx_codec_encode(codec,pixels,255,NULL,0,result)==rows);
        if (rows==1) {
            int bounds[4];cx_codec_repair_bounds(codec,pixels,NULL,255,0,0,128,128,bounds);
            assert(bounds[0]>=bounds[2]);
            pixels[0]=254;cx_codec_repair_bounds(codec,pixels,NULL,255,0,0,128,128,bounds);
            assert(bounds[0]==0&&bounds[1]==0&&bounds[2]==1&&bounds[3]==1);
            pixels[0]=255;
        }
        expected=4000000/rows;assert(cx_codec_rate(codec,4000000)==0);
        pixels[0]^=1;assert(cx_codec_encode(codec,pixels,0,NULL,0,result)>=0);
        cx_codec_close(codec);
    }
    /* A reduced unprotected stream has no native reconstruction certificate.
     * The OpenH264 decoder must not reject its negotiated 1280x720 geometry. */
    free(pixels);pixels=malloc(1920*1080*4);memset(pixels,255,1920*1080*4);
    expected=8000000;void *scaled=cx_codec_open(1920,1080,8000000,1,1);assert(scaled);
    for(int n=0;n<3;n++) {
        struct RowResult result[8];pixels[n*4]=0;
        assert(cx_codec_encode(scaled,pixels,n?0:255,NULL,0,result)==1);
        int bounds[4];cx_codec_repair_bounds(scaled,pixels,NULL,255,0,0,1920,1080,bounds);
        assert(bounds[0]==0&&bounds[1]==0&&bounds[2]==1920&&bounds[3]==1080);
    }
    struct RowResult protected_result[8];struct Rect region={100,100,160,160};
    expected_max_qp=36;
    assert(cx_codec_encode(scaled,pixels,0,&region,1,protected_result)==1);
    assert(!cx_codec_reduced(scaled));
    expected_max_qp=0;
    assert(cx_codec_encode(scaled,pixels,0,NULL,0,protected_result)==1);
    assert(cx_codec_reduced(scaled));
    cx_codec_close(scaled);assert(opens==12&&retunes==9);free(pixels);puts("MP-08/MP-10 runtime OpenH264 rate contract PASS");
}
