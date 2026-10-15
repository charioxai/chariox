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
    Window other=XCreateSimpleWindow(d,DefaultRootWindow(d),256,0,128,160,0,0,0x00ff00);
    XChangeProperty(d,other,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);XMapWindow(d,other);XSync(d,False);
    assert(!cx_capture_open_region(pid,128,160,0,0,128,32));
    struct Capture *selected=cx_capture_open_selected_region(pid,w,128,160,0,0,128,32);assert(selected);
    assert(cx_capture_read(selected,pixels,bounds)==1);for(int i=0;i<128*32;i++)assert(pixels[i*4+2]==255);cx_capture_close(selected);
    assert(!cx_capture_open_selected_region(pid,DefaultRootWindow(d),128,160,0,0,128,32));
    assert(!cx_capture_open_selected_region(pid,w,127,160,0,0,128,32));
    int64_t inventory[32*13];assert(cx_capture_inventory(pid,inventory,32)==2);assert(cx_capture_inventory(pid,inventory,1)==-1);assert(cx_capture_inventory(1,inventory,32)==-1);
    XDestroyWindow(d,other);XSync(d,False);
    /* MP-08/MP-10/MP-11: an owned override-redirect dropdown is a separate
     * whole window. Its pixels/input never fall through to the page below. */
    XSetWindowAttributes popup_attrs={.override_redirect=True,.background_pixel=0x00ff00};
    Window popup=XCreateWindow(d,DefaultRootWindow(d),16,16,65,33,0,CopyFromParent,
        InputOutput,CopyFromParent,CWOverrideRedirect|CWBackPixel,&popup_attrs);
    XChangeProperty(d,popup,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
    XSetTransientForHint(d,popup,w);XMapRaised(d,popup);XSync(d,False);
    struct Capture *aux=cx_capture_open_selected_region(pid,popup,65,33,0,0,64,32);assert(aux);
    assert(cx_capture_read(aux,pixels,bounds)==1);
    for(int i=0;i<64*32;i++)assert(pixels[i*4]==0&&pixels[i*4+1]==255&&pixels[i*4+2]==0);
    assert(pointer_target(aux,10,12,&px,&py)==0);assert(px==26&&py==28);
    assert(!cx_capture_open_selected_region(pid+1,popup,65,33,0,0,64,32));
    XUnmapWindow(d,popup);XSync(d,False);
    assert(cx_capture_read(aux,pixels,bounds)==-1);
    assert(cx_capture_click(aux,10,12)==-1);
    assert(!cx_capture_open_selected_region(pid,popup,65,33,0,0,64,32));
    XMapRaised(d,popup);XSync(d,False);
    assert(cx_capture_read(aux,pixels,bounds)==-1);
    cx_capture_close(aux);
    aux=cx_capture_open_selected_region(pid,popup,65,33,0,0,64,32);assert(aux);
    XSync(aux->display,False);
    XDestroyWindow(d,popup);XSync(d,False);XSync(aux->display,False);
    assert(cx_capture_damage(aux));
    assert(cx_capture_read(aux,pixels,bounds)==-1);
    assert(cx_capture_click(aux,10,12)==-1);cx_capture_close(aux);
    /* Same-size fullscreen hides Chrome UI without changing this crop's
     * dimensions. Page pixels must never become an unmasked Chrome band. */
    struct Capture *fullscreen=cx_capture_open_selected_region(pid,w,128,160,0,0,128,32);assert(fullscreen);
    Atom state_atom=XInternAtom(d,"_NET_WM_STATE",False),full_atom=XInternAtom(d,"_NET_WM_STATE_FULLSCREEN",False);
    XChangeProperty(d,w,state_atom,XA_ATOM,32,PropModeReplace,(unsigned char*)&full_atom,1);XSync(d,False);
    assert(cx_capture_read(fullscreen,pixels,bounds)==-1);
    assert(cx_capture_click(fullscreen,10,10)==-1);
    assert(!cx_capture_open_selected_region(pid,w,128,160,0,0,128,32));
    XDeleteProperty(d,w,state_atom);XSync(d,False);
    assert(cx_capture_read(fullscreen,pixels,bounds)==-1);cx_capture_close(fullscreen);
    struct Capture *transient=cx_capture_open_selected_region(pid,w,128,160,0,0,128,32);assert(transient);
    XSync(transient->display,False);
    XResizeWindow(d,w,128,161);XResizeWindow(d,w,128,160);XSync(d,False);
    assert(cx_capture_damage(transient));
    assert(cx_capture_key_modifiers(transient,XK_l,2)==-1);
    cx_capture_close(transient);
    XResizeWindow(d,w,128,161);XSync(d,False);
    /* MP-11: input can arrive before the next readback processes Configure.
     * An old crop must not target the newly sized window in that interval. */
    assert(cx_capture_click(chrome,10,10)==-1);
    assert(cx_capture_wheel(page,10,10,0,1)==-1);
    assert(cx_capture_key_modifiers(chrome,XK_l,2)==-1);
    assert(cx_capture_read(chrome,pixels,bounds)==-1);
    assert(cx_capture_read(page,pixels,bounds)==-1);
    XResizeWindow(d,w,128,160);XSync(d,False);
    assert(cx_capture_click(chrome,10,10)==-1);
    assert(cx_capture_key_modifiers(chrome,XK_l,2)==-1);
    cx_capture_close(page);cx_capture_close(chrome);
    XFreeGC(d,gc);XDestroyWindow(d,w);XCloseDisplay(d);
    puts("MP-08/MP-10/MP-11 native window-region crop, input origin and retirement PASS");
}
