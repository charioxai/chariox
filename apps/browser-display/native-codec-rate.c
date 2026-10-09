/* MP-08/MP-10: audit bitrate arguments at the real runtime encoder seam. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
struct Rect {int left,top,right,bottom;};
struct RowResult {int row,y,height,key;uint64_t sequence,reference;const uint8_t *bytes;size_t length;};
void *cx_codec_open(int,int,int,int,int);
void cx_codec_close(void *);
int cx_codec_openh264(void *);
int cx_codec_encode(void *,const uint8_t *,unsigned,const struct Rect *,size_t,struct RowResult *);
int cx_codec_rate(void *,int);
void *cx_openh264_open(int,int,int,int,int);
int cx_openh264_rate(void *,int);
static int expected,opens,retunes;
void *audited_openh264_open(int width,int height,int bitrate,int threads,int max_qp) {
    printf("MP-08/MP-10 open bitrate=%d expected=%d\n",bitrate,expected);fflush(stdout);
    assert(bitrate==expected);opens++;
    return cx_openh264_open(width,height,bitrate,threads,max_qp);
}
int audited_openh264_rate(void *encoder,int bitrate) {
    printf("MP-08/MP-10 retune bitrate=%d expected=%d\n",bitrate,expected);fflush(stdout);
    assert(bitrate==expected);retunes++;
    return cx_openh264_rate(encoder,bitrate);
}
int main(void) {
    const int width=1280,height=800;unsigned char *pixels=malloc(width*height*4);
    memset(pixels,255,width*height*4);
    for(int rows=1;rows<=8;rows+=7) {
        struct RowResult result[8];expected=8000000/rows;
        void *codec=cx_codec_open(width,height,8000000,rows,0);assert(codec&&cx_codec_openh264(codec));
        assert(cx_codec_encode(codec,pixels,255,NULL,0,result)==rows);
        expected=4000000/rows;assert(cx_codec_rate(codec,4000000)==0);
        pixels[0]^=1;assert(cx_codec_encode(codec,pixels,0,NULL,0,result)>=0);
        cx_codec_close(codec);
    }
    assert(opens==9&&retunes==9);free(pixels);puts("MP-08/MP-10 runtime OpenH264 rate contract PASS");
}
