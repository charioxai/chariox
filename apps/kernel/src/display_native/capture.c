/* MP-08/MP-10/MP-11: owned-window XShm readback. Events wake, exact bytes
 * authorize damage reuse. No desktop/root pixels, no provider state. */
#include <stdint.h>
#include <time.h>
static double capture_cpu(void) {struct timespec t;clock_gettime(CLOCK_THREAD_CPUTIME_ID,&t);return t.tv_sec*1000.+t.tv_nsec/1000000.;}
#include <stdlib.h>
#include <string.h>
#include <sys/ipc.h>
#include <sys/shm.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/Xatom.h>
#include <X11/extensions/XShm.h>
#include <X11/extensions/Xdamage.h>
#include <X11/extensions/Xcomposite.h>

struct Capture {
    Display *display;
    Window window;
    Pixmap pixmap;
    Damage damage;
    XImage *image;
    XShmSegmentInfo shm;
    int attached, event, width, height, window_height, offset, redirected, redirect_mode;
    unsigned long owner;
    uint8_t *previous;
    /* The slot holding the latest readback. Only this worker writes slots,
     * and a slot is compared before it can be overwritten by a new readback. */
    const uint8_t *last;
    int initialized,motion_height;
    double cpu[3];
    int tile_count,tiles[128][4],adjacent_count,adjacent[128][4];
};
static unsigned long window_pid(Display *d, Window w) {
    Atom actual; int format; unsigned long count, remaining; unsigned char *data = NULL;
    int status = XGetWindowProperty(d, w, XInternAtom(d,"_NET_WM_PID",False),
        0,1,False,XA_CARDINAL,&actual,&format,&count,&remaining,&data);
    unsigned long pid = status == Success && actual == XA_CARDINAL && format == 32 && count == 1 && data ? *(unsigned long *)data : 0;
    if (data) XFree(data);
    return pid;
}
static int dimensions(Display *d, Window w, unsigned *width, unsigned *height) {
    Window root; int x,y; unsigned border,depth;
    return XGetGeometry(d,w,&root,&x,&y,width,height,&border,&depth);
}
void cx_capture_close(struct Capture *c) {
    if (!c) return;
    if (c->display && c->pixmap) XFreePixmap(c->display,c->pixmap);
    if (c->display && c->redirected) XCompositeUnredirectWindow(c->display,c->window,c->redirect_mode);
    if (c->display && c->damage) XDamageDestroy(c->display,c->damage);
    if (c->display && c->attached) { XShmDetach(c->display,&c->shm); XSync(c->display,False); }
    if (c->image) { c->image->data = NULL; XDestroyImage(c->image); }
    if (c->shm.shmaddr && c->shm.shmaddr != (void *)-1) shmdt(c->shm.shmaddr);
    if (c->shm.shmid >= 0) shmctl(c->shm.shmid,IPC_RMID,NULL);
    if (c->display) XCloseDisplay(c->display);
    free(c->previous); free(c);
}
/* MP-08/MP-10/MP-11: the dedicated native worker is the only Xlib caller
 * in this process. Trap only this synchronous request, then restore the
 * previous handler. Another capture/compositor may already own Manual. */
static Display *redirect_display;
static int redirect_error;
static XErrorHandler redirect_previous;
static int redirect_failed(Display *display,XErrorEvent *event) {
    if(display==redirect_display){redirect_error=event->error_code;return 0;}
    return redirect_previous?redirect_previous(display,event):0;
}
static int redirect_window(struct Capture *c,int mode) {
    XSync(c->display,False);redirect_display=c->display;redirect_error=0;
    redirect_previous=XSetErrorHandler(redirect_failed);
    XCompositeRedirectWindow(c->display,c->window,mode);XSync(c->display,False);
    XSetErrorHandler(redirect_previous);redirect_display=NULL;
    if(redirect_error)return 0;
    c->redirected=1;c->redirect_mode=mode;return 1;
}
struct Capture *cx_capture_open(unsigned long owner, int width, int height) {
    struct Capture *c = calloc(1,sizeof(*c));
    if (!c) return NULL;
    c->shm.shmid = -1; c->owner=owner; c->width=width; c->height=height;
    c->display=XOpenDisplay(NULL);
    int error;
    if (!c->display || !XShmQueryExtension(c->display) || !XDamageQueryExtension(c->display,&c->event,&error)) goto fail;
    Window root,parent,*children=NULL; unsigned count=0, found=0;
    if (!XQueryTree(c->display,DefaultRootWindow(c->display),&root,&parent,&children,&count)) goto fail;
    for (unsigned i=0;i<count;i++) {
        unsigned w,h;
        if (window_pid(c->display,children[i])==owner && dimensions(c->display,children[i],&w,&h) && w==(unsigned)width && h>=(unsigned)height && h-height<=400) {
            c->window=children[i]; c->window_height=h; found++;
        }
    }
    if (children) XFree(children);
    if (found!=1) goto fail;
    c->offset=c->window_height-height;
    /* The kernel owns a private X server; its root is never presented. Manual
     * keeps the named owned-window pixmap without compositing an unused root. */
    if(!redirect_window(c,CompositeRedirectManual)&&!redirect_window(c,CompositeRedirectAutomatic))goto fail;
    c->pixmap=XCompositeNameWindowPixmap(c->display,c->window); XSync(c->display,False);
    if (!c->pixmap) goto fail;
    c->image=XShmCreateImage(c->display,DefaultVisual(c->display,0),DefaultDepth(c->display,0),ZPixmap,NULL,&c->shm,width,height);
    if (!c->image || c->image->bits_per_pixel!=32 || c->image->byte_order!=LSBFirst || c->image->bytes_per_line!=width*4 || c->image->red_mask!=0xff0000 || c->image->green_mask!=0xff00 || c->image->blue_mask!=0xff) goto fail;
    size_t size=(size_t)width*height*4;
    c->shm.shmid=shmget(IPC_PRIVATE,size,IPC_CREAT|0600);
    if (c->shm.shmid<0) goto fail;
    c->shm.shmaddr=shmat(c->shm.shmid,NULL,0);
    if (c->shm.shmaddr==(void *)-1) goto fail;
    c->image->data=c->shm.shmaddr;
    if (!XShmAttach(c->display,&c->shm)) goto fail;
    c->attached=1; XSync(c->display,False);
    if (shmctl(c->shm.shmid,IPC_RMID,NULL)) goto fail;
    c->previous=malloc(size);
    if (!c->previous) goto fail;
    c->damage=XDamageCreate(c->display,c->window,XDamageReportRawRectangles);
    return c;
fail:
    cx_capture_close(c); return NULL;
}
void cx_capture_cpu(struct Capture *c,double *out) {memcpy(out,c->cpu,sizeof(c->cpu));}
int cx_capture_fd(struct Capture *c) { return ConnectionNumber(c->display); }
int cx_capture_damage(struct Capture *c) {
    int dirty=0;
    while (XPending(c->display)) {
        XEvent event; XNextEvent(c->display,&event);
        if (event.type==c->event) {
            XRectangle r=((XDamageNotifyEvent *)&event)->area;
            if (r.x<c->width && r.x+r.width>0 && r.y<c->offset+c->height && r.y+r.height>c->offset) dirty=1;
        }
    }
    return dirty;
}
/* MP-08/MP-10/MP-11: exact changed tiles relative to the delivered base. */
int cx_capture_difference(const uint8_t *raw,const uint8_t *previous,int width,int height,int *bounds,int *tile_out,int *motion_height) {
    if(width<1||width>2560||height<1||height>1600)return -2;
    size_t stride=(size_t)width*4;int initialized=previous!=NULL,count=-1,tiles[128][4];
    /* MP-08/MP-10: same bounded CSS input footprint at fourfold Retina pixels. */
    int tile_limit=width==2560&&height==1600?128:32;
    if(motion_height)*motion_height=height;
    int top=height,bottom=0,left=width,right=0,changed_count=0;
    unsigned char changed_rows[1600]={0};
    if (initialized) {
        for (int y=0;y<height;y++) if (memcmp(raw+y*stride,previous+y*stride,stride)) { if (top==height) top=y; bottom=y+1;changed_rows[y]=1;changed_count++; }
        if (top==height) {if(motion_height)*motion_height=0;return 0;}
        if(motion_height)*motion_height=bottom-top;
        if (changed_count<=height*.15) {
            unsigned char marked[80*50]={0};int columns=(width+31)/32;count=0;
            for(int y=top;y<bottom;y++)if(changed_rows[y])for(int x=0;x<width;x+=32){int n=width-x<32?width-x:32;
                if(memcmp(raw+y*stride+x*4,previous+y*stride+x*4,n*4)){int index=y/32*columns+x/32;
                    if(!marked[index]){marked[index]=1;if(count==tile_limit){count=-1;goto dense_tiles;}int *rect=tiles[count++];rect[0]=x;rect[1]=y/32*32;rect[2]=x+n;rect[3]=rect[1]+32<height?rect[1]+32:height;}
                }
            }
        }
        dense_tiles:
        if (changed_count<=height*.15) {
            for (int y=top;y<bottom;y++) for (int x=0;x<width;x+=16) {
                int n=width-x<16?width-x:16;
                if (memcmp(raw+y*stride+x*4,previous+y*stride+x*4,n*4)) { if (x<left)left=x; if (x+n>right)right=x+n; }
            }
        } else { left=0;right=width;top=0;bottom=height; }
    } else { left=0;right=width;top=0;bottom=height; }
    bounds[0]=left;bounds[1]=top;bounds[2]=right;bounds[3]=bottom;
    if(count>0)memcpy(tile_out,tiles,count*4*sizeof(int));
    return count;
}
/* Return -1 on retirement, 0 for byte-identical, 1 for new immutable snapshot.
 * A sparse bound is proved from every row; XDamage bounds never authorize it. */
int cx_capture_read(struct Capture *c, uint8_t *out, int *bounds) {
    c->tile_count=c->adjacent_count=-1;
    memset(c->cpu,0,sizeof(c->cpu));double at=capture_cpu();
    unsigned w,h;
    if (window_pid(c->display,c->window)!=c->owner || !dimensions(c->display,c->window,&w,&h) || w!=(unsigned)c->width || h!=(unsigned)c->window_height || !XShmGetImage(c->display,c->pixmap,c->image,0,c->offset,AllPlanes)) return -1;
    c->cpu[0]=capture_cpu()-at;at=capture_cpu();
    size_t stride=(size_t)c->width*4, size=stride*c->height;
    uint8_t *raw=(uint8_t *)c->image->data;
    /* MP-08/MP-10/MP-11: admission can canonically bind the latest exact
     * readback to the owned base. Equality is proved before rebinding; no
     * serial, damage event or lossy reconstruction supplies this witness. */
    int base_current=c->initialized&&c->last==c->previous;
    int adjacent_bounds[4];
    c->adjacent_count=cx_capture_difference(raw,c->last,c->width,c->height,adjacent_bounds,(int*)c->adjacent,&c->motion_height);
    if(c->adjacent_count==0){
        c->cpu[1]=capture_cpu()-at;at=capture_cpu();memcpy(out,raw,size);if(!base_current)c->last=out;c->cpu[2]=capture_cpu()-at;return 0;
    }
    /* MP-08/MP-10/MP-11: dense motion needs no second full-frame comparison.
     * Sparse cumulative damage alone checks the complete delivered exact base.
     * Adjacent tiles can echo input over a contiguous LOSSY canvas; they never
     * certify its untouched pixels. Dropped captures still use cumulative tiles. */
    if(base_current){
        memcpy(bounds,adjacent_bounds,sizeof(adjacent_bounds));
        c->tile_count=c->adjacent_count;
        if(c->tile_count>0)memcpy(c->tiles,c->adjacent,c->tile_count*4*sizeof(int));
    }else if(c->adjacent_count>0)c->tile_count=cx_capture_difference(raw,c->initialized?c->previous:NULL,c->width,c->height,bounds,(int*)c->tiles,NULL);
    else {bounds[0]=bounds[1]=0;bounds[2]=c->width;bounds[3]=c->height;}
    if(c->tile_count==0){/* Pixels returned to the exact base; ship a conservative full update. */bounds[0]=bounds[1]=0;bounds[2]=c->width;bounds[3]=c->height;c->tile_count=-1;}
    c->cpu[1]=capture_cpu()-at;at=capture_cpu();
    memcpy(out,raw,size);c->last=out;c->cpu[2]=capture_cpu()-at;
    return 1;
}

/* MP-08/MP-10/MP-11: exact sparse damage, not the union's collateral pixels. */
int cx_capture_tiles(struct Capture *c,int *out){if(c->tile_count>0)memcpy(out,c->tiles,c->tile_count*4*sizeof(int));return c->tile_count;}
int cx_capture_adjacent_tiles(struct Capture *c,int *out){if(c->adjacent_count>0)memcpy(out,c->adjacent,c->adjacent_count*4*sizeof(int));return c->adjacent_count;}
void cx_capture_admit(struct Capture *c,const uint8_t *pixels){
    size_t size=(size_t)c->width*c->height*4;
    int current=c->last&&!memcmp(c->last,pixels,size);
    memcpy(c->previous,pixels,size);c->initialized=1;
    /* MP-11: an older/different admission retires the latest-byte shortcut.
     * A following read then conservatively compares as a fresh full capture. */
    c->last=current?c->previous:NULL;
}

/* MP-08/MP-10/MP-11: scheduling only; exact/base bounds stay conservative. */
int cx_capture_motion_height(struct Capture *c){return c->motion_height;}
