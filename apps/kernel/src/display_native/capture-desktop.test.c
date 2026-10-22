/* MP-08/MP-11: owner 0 reads the root of an actual kernel-owned X server. */
#include <assert.h>
#include <stdio.h>
#include <unistd.h>
#include CAPTURE_SOURCE
int main(void) {
    Display *d=XOpenDisplay(NULL);assert(d);
    const int w=1280,h=800;
    /* A root geometry other than the admitted desktop is refused. */
    assert(!cx_capture_open(0,2560,1600));
    struct Capture *c=cx_capture_open(0,w,h);assert(c);
    Window win=XCreateSimpleWindow(d,DefaultRootWindow(d),100,50,64,32,0,0,0x00ff00);
    XMapWindow(d,win);XSync(d,False);
    GC gc=XCreateGC(d,win,0,NULL);XSetForeground(d,gc,0x00ff00);
    XFillRectangle(d,win,gc,0,0,64,32);XSync(d,False);
    for(int n=0;n<100&&!cx_capture_damage(c);n++)usleep(10000);
    uint8_t *pixels=malloc((size_t)w*h*4);int bounds[4];assert(pixels);
    assert(cx_capture_read(c,pixels,bounds)==1);
    const uint8_t *p=pixels+((size_t)(50+10)*w+100+10)*4;
    assert(p[0]==0&&p[1]==255&&p[2]==0);
    /* The root's own pixels outside the window are read too (whole desktop). */
    const uint8_t *outside=pixels+((size_t)700*w+1200)*4;
    assert(!(outside[0]==0&&outside[1]==255&&outside[2]==0));
    /* Desktop input keeps its own admission: native input is refused. */
    assert(cx_capture_click(c,10,10)==-1&&cx_capture_wheel(c,10,10,0,1)==-1&&cx_capture_key(c,'a',0)==-1);
    cx_capture_close(c);
    /* The root survives capture teardown (never freed as a named pixmap). */
    unsigned rw,rh;assert(dimensions(d,DefaultRootWindow(d),&rw,&rh)&&rw==(unsigned)w&&rh==(unsigned)h);
    XFreeGC(d,gc);XDestroyWindow(d,win);XCloseDisplay(d);free(pixels);
    puts("MP-08/MP-11 owned desktop root capture reads every window and refuses native input");
    return 0;
}
