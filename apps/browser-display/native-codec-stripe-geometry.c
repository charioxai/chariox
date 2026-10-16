/* MP-08/MP-10/MP-11: real encoder packets for an independent seam check. */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <assert.h>
struct Rect {int left,top,right,bottom;};
struct RowResult {int row,y,height,key;uint64_t sequence,reference;const uint8_t *bytes;size_t length;};
void *cx_codec_open(int,int,int,int,int);
void cx_codec_close(void *);
int cx_codec_encode(void *,const uint8_t *,unsigned,const struct Rect *,size_t,struct RowResult *);
int main(int argc,char **argv) {
    assert(argc==3);int h=atoi(argv[1]),w=h==1600?2560:1280;
    unsigned char *pixels=malloc((size_t)w*h*4);assert(pixels);
    for(int i=0;i<w*h;i++){pixels[4*i]=pixels[4*i+1]=pixels[4*i+2]=128;pixels[4*i+3]=255;}
    void *codec=cx_codec_open(w,h,8000000,8,0);assert(codec);
    struct RowResult rows[8];int n=cx_codec_encode(codec,pixels,255,NULL,0,rows);assert(n==8);
    int bottom=0;
    for(int i=0;i<n;i++) {
        assert(rows[i].row==i&&rows[i].y==bottom&&rows[i].height>0&&rows[i].key);
        bottom+=rows[i].height;
        char name[4096];assert(snprintf(name,sizeof(name),"%s/%d-row%d.h264",argv[2],h,i)<sizeof(name));
        FILE *file=fopen(name,"wb");assert(file);
        assert(fwrite(rows[i].bytes,1,rows[i].length,file)==rows[i].length);fclose(file);
    }
    assert(bottom==h);cx_codec_close(codec);free(pixels);
}
