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
    cx_capture_close(b);XFreeGC(d,gc);XDestroyWindow(d,w);XCloseDisplay(d);
    puts("MP-08/MP-10/MP-11 manual capture and automatic contention fallback preserve exact owned-window pixels");
}
