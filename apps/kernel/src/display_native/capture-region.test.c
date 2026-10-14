/* MP-08/MP-10/MP-11: crop the owned window, never the root or a neighbour. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include <sys/mman.h>
#include CAPTURE_SOURCE
int main(void) {
    /* Tiny regions must not fingerprint pixels before/after their raster. */
    size_t ps=(size_t)sysconf(_SC_PAGESIZE);
    void *guard=mmap(NULL,ps*3,PROT_NONE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);assert(guard!=MAP_FAILED);
    uint8_t *tiny=(uint8_t*)guard+ps;assert(!mprotect(tiny,ps,PROT_READ|PROT_WRITE));
    memset(tiny,23,8);uint64_t hash;row_hashes(tiny,2,1,&hash);
    munmap(guard,ps*3);
    Display *d=XOpenDisplay(NULL);assert(d);
    Window w=XCreateSimpleWindow(d,DefaultRootWindow(d),0,0,128,160,0,0,0xffffff);
    unsigned long pid=getpid();
    XChangeProperty(d,w,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
    XMapWindow(d,w);XSync(d,False);
    GC gc=XCreateGC(d,w,0,NULL);
    XSetForeground(d,gc,0xff0000);XFillRectangle(d,w,gc,0,0,128,32);
    XSetForeground(d,gc,0x0000ff);XFillRectangle(d,w,gc,0,32,128,128);XSync(d,False);
    struct Capture *chrome=cx_capture_open_region(pid,128,160,0,0,128,32);assert(chrome);
    struct Capture *page=cx_capture_open_region(pid,128,160,16,48,64,64);assert(page);
    uint8_t pixels[128*32*4];int bounds[4];
    assert(cx_capture_read(chrome,pixels,bounds)==1);
    for(int i=0;i<128*32;i++)assert(pixels[i*4]==0&&pixels[i*4+1]==0&&pixels[i*4+2]==255);
    assert(cx_capture_read(page,pixels,bounds)==1);
    for(int i=0;i<64*64;i++)assert(pixels[i*4]==255&&pixels[i*4+1]==0&&pixels[i*4+2]==0);
    assert(!cx_capture_open_region(pid,128,160,127,0,2,32));
    assert(!cx_capture_open_region(pid,128,160,0,-1,128,32));
    assert(!cx_capture_open_region(pid,128,160,0,0,128,161));
    assert(!cx_capture_open_region(1,128,160,0,0,128,32));
    /* The native input transform uses this crop's physical origin. */
    int px,py;assert(pointer_target(page,10,12,&px,&py)==0);assert(px==26&&py==60);
    assert(pointer_target(page,64,12,&px,&py)==-1);
    XSelectInput(d,w,KeyPressMask);XSync(d,False);
    assert(cx_capture_key_modifiers(chrome,XK_l,2)==0);XSync(chrome->display,False);XSync(d,False);
    XEvent e;int chord=0;
    for(int attempt=0;attempt<50&&!chord;attempt++){
        while(XCheckWindowEvent(d,w,KeyPressMask,&e))if(e.xkey.keycode==XKeysymToKeycode(d,XK_l)){
            assert(e.xkey.state&ControlMask);chord++;
        }
        if(!chord){usleep(2000);XSync(d,False);}
    }
    assert(chord==1);
    char keys[32];XQueryKeymap(d,keys);KeyCode control=XKeysymToKeycode(d,XK_Control_L);
    assert(!(keys[control/8]&(1<<(control%8))));
    assert(cx_capture_key_modifiers(chrome,XK_l,16)==-1);
    XResizeWindow(d,w,128,161);XSync(d,False);
    assert(cx_capture_read(chrome,pixels,bounds)==-1);
    assert(cx_capture_read(page,pixels,bounds)==-1);
    cx_capture_close(page);cx_capture_close(chrome);
    XFreeGC(d,gc);XDestroyWindow(d,w);XCloseDisplay(d);
    puts("MP-08/MP-10/MP-11 native window-region crop, input origin and retirement PASS");
}
