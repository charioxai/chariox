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
 Window deep=XCreateSimpleWindow(d,root,0,0,1,1,0,0,0),parent=deep;
 for(int i=0;i<9;i++)parent=XCreateSimpleWindow(d,parent,0,0,1,1,0,0,0);
 XSync(d,False);assert(!cx_capture_open_mode(pid,128,128,1));
 XDestroyWindow(d,deep);
 // Chromium client-side frame insets remain outside the attested viewport.
 XDestroyWindow(d,frame);XSync(d,False);
 frame=XCreateSimpleWindow(d,root,20,20,136,147,0,0,0xffffff);
 client=XCreateSimpleWindow(d,frame,0,0,136,147,0,0,0x0000ff);
 unsigned long insets[4]={4,4,0,4};
 XChangeProperty(d,client,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
 XChangeProperty(d,client,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)insets,4);
 XMapWindow(d,frame);XMapWindow(d,client);XSync(d,False);
 capture=cx_capture_open_mode(pid,128,128,1);assert(capture&&"client frame insets must not enter the viewport");
 gc=XCreateGC(d,client,0,NULL);XSetForeground(d,gc,0xff0000);XFillRectangle(d,client,gc,4,15,128,128);XSync(d,False);
 assert(cx_capture_read(capture,pixels,bounds)==1);
 for(int i=0;i<128*128;i++)assert(pixels[i*4]==0&&pixels[i*4+1]==0&&pixels[i*4+2]==255);
 int px,py;assert(pointer_target(capture,2,3,&px,&py)==0);assert(px==26&&py==38);
 Window covering=XCreateSimpleWindow(d,root,20,20,136,147,0,0,0);XMapRaised(d,covering);XSync(d,False);
 assert(pointer_target(capture,2,3,&px,&py)==1);XDestroyWindow(d,covering);
 insets[0]=5;XChangeProperty(d,client,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)insets,4);XSync(d,False);
 assert(cx_capture_read(capture,pixels,bounds)==-1);assert(pointer_target(capture,2,3,&px,&py)==-1);
 XFreeGC(d,gc);cx_capture_close(capture);
 insets[0]=4;insets[3]=65;XChangeProperty(d,client,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)insets,4);XSync(d,False);
 assert(!cx_capture_open_mode(pid,128,128,1));
 insets[3]=4;XChangeProperty(d,client,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)insets,3);XSync(d,False);
 assert(!cx_capture_open_mode(pid,128,128,1));
#endif
 XDestroyWindow(d,frame);XCloseDisplay(d);
 puts("MP-08/MP-10/MP-11 reparented owned client capture, root visibility and foreign/ambiguous refusal PASS");
}
