/* MP-08/MP-10/MP-11: real X reparenting, PID admission and visible root pixels.
 * Supplementary native regression; hosted/public-site acceptance is separate. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include CAPTURE_SOURCE
int main(void) {
 Display *d=XOpenDisplay(NULL);assert(d);Window root=DefaultRootWindow(d);
 Window frame=XCreateSimpleWindow(d,root,20,20,128,128,0,0,0xffffff);
 Window client=XCreateSimpleWindow(d,frame,0,0,128,128,0,0,0xff0000);
 unsigned long pid=getpid();XChangeProperty(d,client,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
 XMapWindow(d,frame);XMapWindow(d,client);XSync(d,False);
 struct Capture *capture=cx_capture_open(pid,128,128);assert(capture&&"reparented Chromium client must be found beneath its WM frame");
 cx_capture_close(capture);
#ifdef TEST_AUTOMATIC
 capture=cx_capture_open_mode(pid,128,128,1);assert(capture);
 GC gc=XCreateGC(d,client,0,NULL);XSetForeground(d,gc,0xff0000);XFillRectangle(d,client,gc,0,0,128,128);XSync(d,False);
 uint8_t pixels[128*128*4];int bounds[4];assert(cx_capture_read(capture,pixels,bounds)==1);
 for(int i=0;i<128*128;i++)assert(pixels[i*4]==0&&pixels[i*4+1]==0&&pixels[i*4+2]==255);
 XImage *visible=XGetImage(d,root,20,20,128,128,AllPlanes,ZPixmap);assert(visible);
 assert(XGetPixel(visible,64,64)==0xff0000&&"Browser capture must keep Chromium visible on the Computer root");
 XDestroyImage(visible);XFreeGC(d,gc);cx_capture_close(capture);
 // A second same-PID client is ambiguous; a foreign PID never qualifies.
 Window duplicate=XCreateSimpleWindow(d,root,200,20,128,128,0,0,0);
 XChangeProperty(d,duplicate,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);XMapWindow(d,duplicate);XSync(d,False);
 assert(!cx_capture_open_mode(pid,128,128,1));assert(!cx_capture_open_mode(pid+100000,128,128,1));
 XDestroyWindow(d,duplicate);
#endif
 XDestroyWindow(d,frame);XCloseDisplay(d);
 puts("MP-08/MP-10/MP-11 reparented owned client capture, root visibility and foreign/ambiguous refusal PASS");
}
