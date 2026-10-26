/* MP-08/MP-10/MP-11: the owned-surface fence on a real X server. A steady readback sends only the
 * image read; every change the binding depends on (PID, state, insets, size, mapping of the window or
 * an ancestor, destruction) is reported by the server and refuses the next readback. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include CAPTURE_SOURCE
static unsigned long pid;
static Window make(Display *d,Window parent,int w,int h) {
 Window window=XCreateSimpleWindow(d,parent,0,0,w,h,0,0,0xffffff);
 XChangeProperty(d,window,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
 XMapWindow(d,window);return window;
}
static uint8_t pixels[128*128*4];static int bounds[4];
static unsigned long sent(struct Capture *c,int *result) {
 unsigned long before=XNextRequest(c->display);*result=cx_capture_read(c,pixels,bounds);return XNextRequest(c->display)-before;
}
#define REFUSES(name,...) do { \
 Window parent=make(d,DefaultRootWindow(d),160,160),window=XCreateSimpleWindow(d,parent,0,0,128,128,0,0,0xffffff),victim=window,frame=parent; \
 (void)frame;(void)victim;(void)window; \
 XChangeProperty(d,window,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);XMapWindow(d,window);XSync(d,False); \
 struct Capture *c=cx_capture_open_mode(pid,128,128,1);assert(c&&name); \
 int r;assert(cx_capture_read(c,pixels,bounds)==1); \
 for(int i=0;i<4;i++){unsigned long n=sent(c,&r);assert(r>=0);assert(n<=1&&"steady readback sends the image request only");} \
 __VA_ARGS__;XSync(d,False); \
 sent(c,&r);assert(r==-1&&name);sent(c,&r);assert(r==-1&&"a refused binding stays retired"); \
 cx_capture_close(c);XDestroyWindow(d,parent);XSync(d,False); \
} while(0)
int main(void) {
 Display *d=XOpenDisplay(NULL);assert(d);pid=getpid();
 REFUSES("owner PID change",{unsigned long other=pid+1;XChangeProperty(d,window,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&other,1);});
 REFUSES("fullscreen state",{Atom a[1]={XInternAtom(d,"_NET_WM_STATE_FULLSCREEN",False)};XChangeProperty(d,window,XInternAtom(d,"_NET_WM_STATE",False),XA_ATOM,32,PropModeReplace,(unsigned char*)a,1);});
 REFUSES("frame insets",{unsigned long e[4]={5,0,0,0};XChangeProperty(d,window,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)e,4);});
 REFUSES("window resize",XResizeWindow(d,window,129,128));
 REFUSES("window unmapped",XUnmapWindow(d,window));
 REFUSES("ancestor unmapped",XUnmapWindow(d,parent));
 REFUSES("window destroyed",XDestroyWindow(d,window));
 { /* A title change is not part of the binding: no recheck, no refusal. */
  Window window=make(d,DefaultRootWindow(d),128,128);XSync(d,False);
  struct Capture *c=cx_capture_open_mode(pid,128,128,1);assert(c);int r;assert(cx_capture_read(c,pixels,bounds)==1);
  XStoreName(d,window,"title");XSync(d,False);assert(sent(c,&r)<=1&&r>=0);
  XDestroyWindow(d,window);cx_capture_close(c);
 }
 XCloseDisplay(d);puts("MP-08/MP-10/MP-11 owned-surface fence: steady readback sends one request; every dependent change refuses the next readback PASS");
}
