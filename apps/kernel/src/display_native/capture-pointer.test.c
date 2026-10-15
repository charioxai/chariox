/* MP-11 (review #893 P2): actual private X server. A top-level window over
 * part of the owned window (a Chromium bubble/popup outside the capture)
 * must not receive native pointer input: the click/wheel is refused. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include <X11/Xlib.h>
#include CAPTURE_SOURCE
static int presses(Display *d,Window w,struct Capture *c){
    /* MP-11: order the injector connection before reading another client's
     * event queue. XFlush alone does not establish that cross-client order. */
    int n=0;XEvent e;XSync(c->display,False);XSync(d,False);
    while(XCheckWindowEvent(d,w,ButtonPressMask,&e))n++;
    return n;
}
/* Protocol 530: the button of the single press, and the release point of a drag. */
static int pressed_button(Display *d,Window w,struct Capture *c){
    int button=0,n=0;XEvent e;XSync(c->display,False);XSync(d,False);
    while(XCheckWindowEvent(d,w,ButtonPressMask,&e)){n++;button=e.xbutton.button;}
    return n==1?button:-1;
}
static int released_at(Display *d,Window w,struct Capture *c,int *x,int *y){
    int n=0,motions=0;XEvent e;XSync(c->display,False);XSync(d,False);
    while(XCheckWindowEvent(d,w,ButtonReleaseMask|Button1MotionMask,&e)){
        if(e.type==MotionNotify)motions++;
        else if(e.xbutton.button==1){n++;*x=e.xbutton.x;*y=e.xbutton.y;}
    }
    return n==1?motions:-1;
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
    assert(cx_capture_click(c,10,10)==0);assert(presses(d,w,c)==1);
    assert(cx_capture_click(c,70,70)==1);assert(presses(d,popup,c)==0);assert(presses(d,w,c)==0);
    assert(cx_capture_wheel(c,70,70,0,1)==1);assert(presses(d,popup,c)==0);
    assert(cx_capture_wheel(c,10,10,0,1)==0);assert(presses(d,w,c)==1); /* one notch: one button-5 press */
    /* Protocol 530: middle and right buttons, and a drag that may not end under a foreign window. */
    XSelectInput(d,w,ButtonPressMask|ButtonReleaseMask|Button1MotionMask);XSync(d,False);
    assert(cx_capture_button(c,10,10,2)==0);assert(pressed_button(d,w,c)==2);
    assert(cx_capture_button(c,10,10,3)==0);assert(pressed_button(d,w,c)==3);
    assert(cx_capture_button(c,10,10,4)==-1&&cx_capture_button(c,10,10,0)==-1);assert(pressed_button(d,w,c)==-1);
    { int x=0,y=0;released_at(d,w,c,&x,&y);
      assert(cx_capture_drag(c,10,10,70,70)==1);assert(pressed_button(d,w,c)==-1);
      assert(cx_capture_drag(c,10,10,50,20)==0);assert(released_at(d,w,c,&x,&y)>=4);assert(x==50&&y==20);assert(presses(d,w,c)==1); }
    XSelectInput(d,w,ButtonPressMask);XSync(d,False);
    XUnmapWindow(d,popup);XSync(d,False);
    assert(cx_capture_click(c,70,70)==0);assert(presses(d,w,c)==1);
    cx_capture_close(c);XDestroyWindow(d,popup);XDestroyWindow(d,w);XCloseDisplay(d);
    puts("MP-11 native pointer input reaches only the uncovered owned window");
}
