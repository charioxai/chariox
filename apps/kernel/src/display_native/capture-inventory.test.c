/* L1: the persistent inventory connection on a real X server. Idle costs no events; every change a
 * row depends on (a popup mapped, frame extents, focus, geometry, unmapping) is reported at once and
 * listed by the next read; a title change is not a trigger. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include CAPTURE_SOURCE
static unsigned long pid;
static Window make(Display *d,int override,int x,int y,int w,int h) {
 XSetWindowAttributes a={.override_redirect=override};
 Window window=XCreateWindow(d,DefaultRootWindow(d),x,y,w,h,0,CopyFromParent,InputOutput,CopyFromParent,CWOverrideRedirect,&a);
 XChangeProperty(d,window,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
 XMapWindow(d,window);return window;
}
static int64_t rows[32*13];
static int listed(struct Inventory *v,Window w) {
 int n=cx_inventory_read(v,rows,32);assert(n>=0);
 for(int i=0;i<n;i++)if((Window)rows[i*13]==w)return 1;return 0;
}
/* Milliseconds until the connection reports a change (or -1). */
static int wait_changed(struct Inventory *v,int limit_ms) {
 struct timespec t0,t1;clock_gettime(CLOCK_MONOTONIC,&t0);
 if(cx_inventory_wait(v,-1,limit_ms)!=1)return -1;
 clock_gettime(CLOCK_MONOTONIC,&t1);
 return (int)((t1.tv_sec-t0.tv_sec)*1000+(t1.tv_nsec-t0.tv_nsec)/1000000);
}
#define QUIET(v) do { int64_t _r[13];(void)_r;usleep(300000);assert(cx_inventory_changed(v)==0&&"no event while nothing changed"); } while(0)
int main(void) {
 Display *d=XOpenDisplay(NULL);assert(d);pid=getpid();
 Window top=make(d,0,0,0,200,150);XSync(d,False);
 struct Inventory *v=cx_inventory_open(pid);assert(v);
 assert(listed(v,top));
 cx_inventory_changed(v);
 /* Idle second: nothing to read. */
 usleep(1000000);assert(cx_inventory_changed(v)==0&&"an idle display reports no change");
 /* A popup (override-redirect) mapped: reported within 20 ms and listed. */
 Window popup=make(d,1,10,10,80,60);XSync(d,False);
 int ms=wait_changed(v,200);assert(ms>=0&&ms<=20&&"popup reported within 20 ms");
 assert(listed(v,popup));
 /* Walk-selected events: drained after the read, then quiet. */
 usleep(100000);cx_inventory_changed(v);QUIET(v);
 /* Title change is not part of any row. */
 XStoreName(d,top,"title");XSync(d,False);usleep(100000);assert(cx_inventory_changed(v)==0&&"a title change is no trigger");
 /* Frame extents on an owner window. */
 unsigned long e[4]={4,4,4,4};
 XChangeProperty(d,top,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)e,4);XSync(d,False);
 assert(wait_changed(v,200)>=0&&"frame extents change reported");
 assert((int)rows[0]>=0);assert(cx_inventory_read(v,rows,32)>=2);
 usleep(100000);cx_inventory_changed(v);
 /* Geometry of an owner window. */
 XResizeWindow(d,top,210,150);XSync(d,False);assert(wait_changed(v,200)>=0&&"resize reported");
 cx_inventory_read(v,rows,32);usleep(100000);cx_inventory_changed(v);
 /* Focus. */
 XSetInputFocus(d,top,RevertToParent,CurrentTime);XSync(d,False);assert(wait_changed(v,200)>=0&&"focus change reported");
 cx_inventory_read(v,rows,32);usleep(100000);cx_inventory_changed(v);
 /* Unmapped popup. */
 XUnmapWindow(d,popup);XSync(d,False);assert(wait_changed(v,200)>=0&&"popup unmapped reported");
 XDestroyWindow(d,popup);XSync(d,False);usleep(50000);cx_inventory_changed(v);
 /* A window destroyed right after being found must not kill the process (BadWindow on the select). */
 for(int i=0;i<20;i++){Window w=make(d,1,0,0,20,20);XSync(d,False);XDestroyWindow(d,w);cx_inventory_read(v,rows,32);}
 XSync(d,False);cx_inventory_changed(v);
 /* A popup whose MapNotify a concurrent walk already read into Xlib's queue (socket now empty) still wakes the waiter. */
 Window late=make(d,1,30,30,40,40);XSync(d,False);usleep(50000);
 cx_inventory_read(v,rows,32);
 assert(wait_changed(v,200)>=0&&"an event buffered by a walk wakes the waiter without a later X event");
 XDestroyWindow(d,late);XSync(d,False);usleep(50000);cx_inventory_changed(v);
 cx_inventory_close(v);XCloseDisplay(d);
 puts("L1 inventory connection: idle quiet, popup within 20 ms, extents/geometry/focus/unmap reported, title ignored PASS");
}
