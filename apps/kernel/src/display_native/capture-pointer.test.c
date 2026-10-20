/* MP-11 (review #893 P2): actual private X server. A top-level window over
 * part of the owned window (a Chromium bubble/popup outside the capture)
 * must not receive native pointer input: the click/wheel is refused. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include <X11/Xlib.h>
#include CAPTURE_SOURCE
static int presses(Display *d,Window w){
    int n=0;XEvent e;XSync(d,False);
    while(XCheckWindowEvent(d,w,ButtonPressMask,&e))n++;
    return n;
}
int main(void) {
    Display *d=XOpenDisplay(NULL);assert(d);
    Window w=XCreateSimpleWindow(d,DefaultRootWindow(d),0,0,128,128,0,0,0xffffff);
    unsigned long pid=getpid();XChangeProperty(d,w,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
    XSelectInput(d,w,ButtonPressMask);XMapWindow(d,w);XSync(d,False);
    XSetWindowAttributes a={.override_redirect=True};
    Window popup=XCreateWindow(d,DefaultRootWindow(d),60,60,40,40,0,CopyFromParent,InputOutput,CopyFromParent,CWOverrideRedirect,&a);
    XSelectInput(d,popup,ButtonPressMask);XMapRaised(d,popup);XSync(d,False);
    struct Capture *c=cx_capture_open(pid,128,128);assert(c);
    assert(cx_capture_click(c,10,10)==0);assert(presses(d,w)==1);
    assert(cx_capture_click(c,70,70)==1);assert(presses(d,popup)==0);assert(presses(d,w)==0);
    assert(cx_capture_wheel(c,70,70,0,1)==1);assert(presses(d,popup)==0);
    assert(cx_capture_wheel(c,10,10,0,1)==0);assert(presses(d,w)==1); /* one notch: one button-5 press */
    XUnmapWindow(d,popup);XSync(d,False);
    assert(cx_capture_click(c,70,70)==0);assert(presses(d,w)==1);
    cx_capture_close(c);XDestroyWindow(d,popup);XDestroyWindow(d,w);XCloseDisplay(d);
    puts("MP-11 native pointer input reaches only the uncovered owned window");
}
