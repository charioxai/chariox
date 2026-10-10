/* MP-08/MP-10/MP-11: supplementary readback-state regression. The X replies
 * below provide geometry only; real capture/relay/client gates run separately.
 * Compile with the native-display X headers/libs; output stays outside source. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
static size_t row_bytes;
static int row_compares;
static int counted_compare(const void *a,const void *b,size_t n){
    if(n==row_bytes)row_compares++;
    return memcmp(a,b,n);
}
#define memcmp counted_compare
#include "capture.c"
#undef memcmp

/* The raster is supplied directly, without launching an X server. */
Atom XInternAtom(Display *d,const char *name,Bool only){(void)d;(void)name;(void)only;return 1;}
int XGetWindowProperty(Display *d,Window w,Atom p,long off,long len,Bool del,Atom requested,
    Atom *actual,int *format,unsigned long *count,unsigned long *remaining,unsigned char **data){
    (void)d;(void)w;(void)p;(void)off;(void)len;(void)del;(void)requested;
    *actual=XA_CARDINAL;*format=32;*count=1;*remaining=0;
    unsigned long *pid=malloc(sizeof(*pid));*pid=42;*data=(unsigned char*)pid;return Success;
}
Status XGetGeometry(Display *d,Drawable a,Window *root,int *x,int *y,unsigned *w,unsigned *h,unsigned *border,unsigned *depth){
    (void)d;(void)a;*root=0;*x=*y=0;*w=1280;*h=800;*border=0;*depth=24;return 1;
}
Bool XShmGetImage(Display *d,Drawable a,XImage *image,int x,int y,unsigned long plane){
    (void)d;(void)a;(void)image;(void)x;(void)y;(void)plane;return True;
}
int main(void){
    const int w=1280,h=800;const size_t size=(size_t)w*h*4;row_bytes=w*4;
    uint8_t *raw=malloc(size),*a=malloc(size),*b=malloc(size),*c=malloc(size);
    assert(raw&&a&&b&&c);memset(raw,255,size);
    XImage image={.data=(char*)raw};struct Capture capture={.owner=42,.width=w,.height=h,.window_height=h,.image=&image,.previous=malloc(size)};
    assert(capture.previous);int bounds[4];
    assert(cx_capture_read(&capture,a,bounds)==1);
    cx_capture_admit(&capture,a);
    /* An unchanged read between admission and input must preserve its witness. */
    assert(cx_capture_read(&capture,b,bounds)==0);
    raw[(20*w+10)*4]=0;row_compares=0;
    assert(cx_capture_read(&capture,b,bounds)==1);
    assert(capture.tile_count==1&&capture.tiles[0][0]==0&&capture.tiles[0][1]==0);
    assert(bounds[0]==0&&bounds[1]==20&&bounds[2]==16&&bounds[3]==21);
    int single_scan=row_compares;
    /* B was never admitted. C must include both disjoint changes since A. */
    raw[(780*w+1270)*4]=0;
    assert(cx_capture_read(&capture,c,bounds)==1);
    assert(capture.tile_count==2&&capture.tiles[1][0]==1248&&capture.tiles[1][1]==768);
    assert(bounds[0]==0&&bounds[1]==20&&bounds[2]==1280&&bounds[3]==781);
    /* An old admission is not a witness for the latest raster: force a full
     * update even though the following readback still has C's identical bytes. */
    cx_capture_admit(&capture,b);
    assert(cx_capture_read(&capture,a,bounds)==1);
    assert(capture.tile_count==-1&&bounds[0]==0&&bounds[1]==0&&bounds[2]==w&&bounds[3]==h);
    cx_capture_admit(&capture,a);
    assert(cx_capture_read(&capture,c,bounds)==0);
    printf("MP-08/MP-10/MP-11 row comparisons after current admission: %d (expected %d); dropped/stale admission damage complete\n",single_scan,h);
    free(capture.previous);free(raw);free(a);free(b);free(c);
    assert(single_scan==h);return 0;
}
