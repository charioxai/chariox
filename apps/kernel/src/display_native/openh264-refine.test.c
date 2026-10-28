/* MP-08/MP-10: a page IDR spends its budget and the unchanged picture then
 * refines at the negotiated rate. Built against the production encoder source. */
#include "openh264.c"
#include <assert.h>
#include <stdio.h>

int main(void) {
    const int width=1920,height=1080,bitrate=8000000,frames=60;
    if (!cx_openh264_available()) {puts("MP-08/MP-10 OpenH264 runtime missing");return 2;}
    /* A text page: white, with lines of glyph-sized ink boxes. */
    uint8_t *y=malloc((size_t)width*height),*u=malloc((size_t)width*height/4),*v=malloc((size_t)width*height/4);
    assert(y&&u&&v);
    memset(y,235,(size_t)width*height);memset(u,128,(size_t)width*height/4);memset(v,128,(size_t)width*height/4);
    uint32_t seed=7;
    for (int line=40;line+12<height;line+=26) for (int x=240;x+8<1240;x+=9) {
        seed=seed*1103515245+12345;
        if ((seed>>16)%7==0) continue;
        int glyph=4+(seed>>8)%5;
        /* Two stems and a bar, like a letter. */
        for (int r=0;r<10;r++) for (int c=0;c<glyph;c++) if (c==0||c==glyph-1||r==4+(seed>>12)%3) y[(size_t)(line+r)*width+x+c]=16;
    }
    uint8_t *planes[3]={y,u,v};int strides[3]={width,width/2,width/2};
    double model[2]={0.02,0.02};
    struct Encoder *h=cx_openh264_open(width,height,bitrate,1,0,model);
    assert(h);
    uint8_t *packet=NULL;size_t capacity=0,total=0;int key=0,idr_qp=0,refined_at=-1,idr_bytes=0;
    for (int n=0;n<frames;n++) {
        int length=cx_openh264_encode(h,planes,strides,width,height,(uint64_t)n,&packet,&capacity,&key);
        assert(length>0);
        assert(key==(n==0));
        total+=length;
        if (!n) {idr_qp=h->qp;idr_bytes=length;}
        if (refined_at<0&&h->qp<=30) refined_at=n;
    }
    const double frame_bytes=bitrate/60./8;
    printf("MP-08/MP-10 page IDR qp=%d bytes=%d bound=%d refined_at=%d total=%zu budget=%.0f\n",
        idr_qp,idr_bytes,cx_openh264_key_bytes(h),refined_at,total,(CX_IDR_FRAMES+frames)*frame_bytes);fflush(stdout);
    /* The IDR is not the QP 51 a 1.5-frame VBV forced, and it stays within the key bound. */
    assert(idr_qp<CX_QP_MAX&&idr_bytes<=cx_openh264_key_bytes(h));
    /* The unchanged picture refines to QP 30 within 12 frames (200 ms at 60 fps)... */
    assert(refined_at>0&&refined_at<=12);
    /* ...and the whole run stays within the IDR budget plus the rate, with one frame of slack. */
    assert(total<=(CX_IDR_FRAMES+frames+1)*frame_bytes);
    cx_openh264_close(h);
    /* A key of the old (blank) window followed by the new document's paint: the paint gets the budget instead. */
    uint8_t *blank=malloc((size_t)width*height);assert(blank);memset(blank,235,(size_t)width*height);
    uint8_t *blank_planes[3]={blank,u,v};
    h=cx_openh264_open(width,height,bitrate,1,0,model);assert(h);
    assert(cx_openh264_encode(h,blank_planes,strides,width,height,0,&packet,&capacity,&key)>0&&key);
    int painted=cx_openh264_encode(h,planes,strides,width,height,1,&packet,&capacity,&key);
    printf("MP-08/MP-10 first paint after a blank key qp=%d bytes=%d\n",h->qp,painted);fflush(stdout);
    assert(painted>0&&!key&&h->qp<CX_QP_MAX);
    cx_openh264_close(h);free(packet);free(blank);free(y);free(u);free(v);
    puts("MP-08/MP-10 OpenH264 page IDR budget and refinement PASS");
    return 0;
}
