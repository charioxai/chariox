/* MP-08/MP-10/MP-11: actual private X server; no desktop/provider state. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include <X11/Xlib.h>
#include <X11/extensions/Xcomposite.h>
static int manual,automatic;
static void track_redirect(Display *d,Window w,int mode){
    if(mode==CompositeRedirectManual)manual++;else automatic++;
    XCompositeRedirectWindow(d,w,mode);
}
#define XCompositeRedirectWindow track_redirect
#include CAPTURE_SOURCE
#undef XCompositeRedirectWindow
static unsigned root_pixel(Display *d,int x,int y){XImage *i=XGetImage(d,DefaultRootWindow(d),x,y,1,1,AllPlanes,ZPixmap);unsigned p=XGetPixel(i,0,0);XDestroyImage(i);return p&0xffffff;}
int main(void) {
    Display *d=XOpenDisplay(NULL);assert(d);
    Window w=XCreateSimpleWindow(d,DefaultRootWindow(d),0,0,128,128,0,0,0xffffff);
    unsigned long pid=getpid();XChangeProperty(d,w,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
    XMapWindow(d,w);XSync(d,False);
    struct Capture *a=cx_capture_open(pid,128,128);assert(a);
    assert(manual==1&&automatic==0);
    struct Capture *b=cx_capture_open(pid,128,128);assert(b);
    assert(manual==2&&automatic==1); /* Only one Manual owner is allowed. */
    GC gc=XCreateGC(d,w,0,NULL);XSetForeground(d,gc,0xff0000);
    XFillRectangle(d,w,gc,0,0,128,128);XSync(d,False);
    uint8_t pixels[128*128*4];int bounds[4];
    assert(cx_capture_read(a,pixels,bounds)==1);
    for(int i=0;i<128*128;i++)assert(pixels[i*4]==0&&pixels[i*4+1]==0&&pixels[i*4+2]==255);
    cx_capture_close(a);
    assert(cx_capture_read(b,pixels,bounds)==1);
    for(int i=0;i<128*128;i++)assert(pixels[i*4]==0&&pixels[i*4+1]==0&&pixels[i*4+2]==255);
    cx_capture_close(b);XFreeGC(d,gc);XDestroyWindow(d,w);XSync(d,False);
    /* O1: on the owned desktop a capture with no root consumer leaves the root unpainted (Manual);
     * a consumer that attaches later sees the window's current pixels at once, and a capture
     * opened after it is Automatic from the start. */
    setenv("CHARIOX_OWNED_VIRTUAL_DISPLAY","1",1);
    manual=automatic=0;cx_capture_set_root_presented(0);
    Window w2=XCreateSimpleWindow(d,DefaultRootWindow(d),200,200,128,128,0,0,0xffffff);
    XChangeProperty(d,w2,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
    XMapWindow(d,w2);XSync(d,False);
    struct Capture *e=cx_capture_open(pid,128,128);assert(e);
    assert(manual==1&&automatic==0);
    GC g2=XCreateGC(d,w2,0,NULL);XSetForeground(d,g2,0xff0000);XFillRectangle(d,w2,g2,0,0,128,128);XSync(d,False);
    assert(root_pixel(d,210,210)!=0xff0000);
    assert(cx_capture_present_root(e)==1);
    assert(root_pixel(d,210,210)==0xff0000);
    XSetForeground(d,g2,0x0000ff);XFillRectangle(d,w2,g2,0,0,128,128);XSync(d,False);
    assert(root_pixel(d,210,210)==0x0000ff);
    assert(cx_capture_read(e,pixels,bounds)==1);
    for(int i=0;i<128*128;i++)assert(pixels[i*4]==255&&pixels[i*4+1]==0&&pixels[i*4+2]==0);
    assert(cx_capture_present_root(e)==1);
    cx_capture_close(e);
    manual=automatic=0;cx_capture_set_root_presented(1);
    struct Capture *f=cx_capture_open(pid,128,128);assert(f);
    assert(manual==0&&automatic==1);
    cx_capture_close(f);XFreeGC(d,g2);XDestroyWindow(d,w2);XCloseDisplay(d);
    puts("MP-08/MP-10/MP-11 manual capture and automatic contention fallback preserve exact owned-window pixels; O1 root presentation switches without a stale root frame");
}
