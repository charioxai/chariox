/* MP-08/MP-10/MP-11: owned-window XShm readback. Events wake, exact bytes
 * authorize damage reuse. Owner 0 reads the root of the kernel-owned desktop
 * X server (never a login session); the kernel masks it before any encode.
 * No provider state. */
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <time.h>
static double capture_cpu(void) {struct timespec t;clock_gettime(CLOCK_THREAD_CPUTIME_ID,&t);return t.tv_sec*1000.+t.tv_nsec/1000000.;}
#include <stdio.h>
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
#include <X11/extensions/XTest.h>
#include <X11/XKBlib.h>
#include <X11/keysym.h>

struct Capture {
    Display *display;
    Window window;
    Pixmap pixmap;
    Damage damage;
    XImage *image;
    XShmSegmentInfo shm;
    int attached, event, width, height, window_width, window_height, xoffset, offset, redirected, redirect_mode, desktop, retired;
    unsigned long insets[4];
    unsigned long owner;
    Atom state_atom,fullscreen_atom;
    /* Owned-surface fence: proved once, then re-proved only after the server reports a change to
     * the window, its ancestors or the three properties the binding reads. */
    Atom pid_atom,insets_atom;
    Window ancestors[16];int ancestor_count,surface_dirty,woken;
    uint8_t *previous;
    /* The slot holding the latest readback. Only this worker writes slots,
     * and a slot is compared before it can be overwritten by a new readback. */
    const uint8_t *last;
    int initialized,motion_height;
    double cpu[3];
    int tile_count,tiles[128][4],adjacent_count,adjacent[128][4];
    /* MP-08/MP-10: vertical scroll decomposition relative to the previous
     * readback. Row hashes only vote for a candidate offset; every reported
     * cell is proved with memcmp. Other bases are planned by the worker. */
    struct Shift {int valid,dy,moves,dirty,dirty_pixels,move[64][4],dirt[64][4];} shift;
    uint64_t *hash_cur,*hash_last;int hash_last_valid,last_dy,plans;
    unsigned char state[1600][40];
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
/* MP-08/MP-10/MP-11: Chromium publishes its client-side frame/shadow
 * extents. They select a bounded candidate crop, proved by CDP attestation
 * before admission and re-checked before every readback and native input. */
static int frame_insets(Display *d,Window w,unsigned long insets[4]) {
    Atom actual;int format;unsigned long count,remaining;unsigned char *data=NULL;
    memset(insets,0,4*sizeof(*insets));
    int status=XGetWindowProperty(d,w,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),0,4,False,XA_CARDINAL,&actual,&format,&count,&remaining,&data);
    int valid=status==Success&&(actual==None||(actual==XA_CARDINAL&&format==32&&count==4&&!remaining&&data));
    if(valid&&actual!=None)for(int i=0;i<4;i++){insets[i]=((unsigned long*)data)[i];if(insets[i]>64)valid=0;}
    if(data)XFree(data);
    return valid;
}
/* MP-11: a WM can hide Chrome UI at unchanged window dimensions. All
 * window-backed crops then lose their page/Chrome origin; desktop-root
 * capture retains its ordinary protection path. Malformed state refuses. */
static int normal_window(struct Capture *c) {
    if(!c->state_atom)c->state_atom=XInternAtom(c->display,"_NET_WM_STATE",False);
    if(!c->fullscreen_atom)c->fullscreen_atom=XInternAtom(c->display,"_NET_WM_STATE_FULLSCREEN",False);
    Atom actual;int format;unsigned long count,remaining;unsigned char *data=NULL;
    int status=XGetWindowProperty(c->display,c->window,c->state_atom,0,64,False,XA_ATOM,&actual,&format,&count,&remaining,&data);
    int normal=status==Success&&!remaining&&(actual==None||(actual==XA_ATOM&&format==32&&count<=64&&(!count||data)));
    if(normal&&actual!=None)for(unsigned long i=0;i<count;i++)if(((Atom*)data)[i]==c->fullscreen_atom)normal=0;
    if(data)XFree(data);
    return normal;
}
static int bound_surface(struct Capture *c) {
    XWindowAttributes attributes;unsigned long insets[4];
    if(!XGetWindowAttributes(c->display,c->window,&attributes)||attributes.map_state!=IsViewable||
        attributes.width!=c->window_width||attributes.height!=c->window_height)return 0;
    return c->desktop||(window_pid(c->display,c->window)==c->owner&&normal_window(c)&&frame_insets(c->display,c->window,insets)&&!memcmp(insets,c->insets,sizeof(insets)));
}
void cx_capture_close(struct Capture *c) {
    if (!c) return;
    if (c->display && c->pixmap && !c->desktop) XFreePixmap(c->display,c->pixmap);
    if (c->display && c->redirected && !c->retired) XCompositeUnredirectWindow(c->display,c->window,c->redirect_mode);
    if (c->display && c->damage && !c->retired) XDamageDestroy(c->display,c->damage);
    if (c->display && c->attached) { XShmDetach(c->display,&c->shm); XSync(c->display,False); }
    if (c->image) { c->image->data = NULL; XDestroyImage(c->image); }
    if (c->shm.shmaddr && c->shm.shmaddr != (void *)-1) shmdt(c->shm.shmaddr);
    if (c->shm.shmid >= 0) shmctl(c->shm.shmid,IPC_RMID,NULL);
    if (c->display) XCloseDisplay(c->display);
    free(c->previous); free(c->hash_cur); free(c->hash_last); free(c);
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
/* MP-08/MP-10: fixed-label refusal stage plus owned-window geometry on the
 * worker's private stderr channel. Sizes only; no titles, pixels or IDs. */
static void open_refused(const char *stage) {fprintf(stderr,"MD-DISPLAY: native stage %s\n",stage);}
/* MP-08/MP-10/MP-11: a window manager reparents the owned client beneath
 * its frame. Search a bounded tree; capture only the exact PID/geometry,
 * never the manager frame. A partial/ambiguous search grants no capture. */
struct ClientSearch {struct Capture *capture;Window selected;int window_width,window_height;unsigned remaining,found,owned;int failed;char seen[256];};
static void find_client_windows(struct ClientSearch *search,Window parent,unsigned depth) {
    Window root,ancestor,*children=NULL;unsigned count=0;
    if(!XQueryTree(search->capture->display,parent,&root,&ancestor,&children,&count)){search->failed=1;return;}
    if(count>search->remaining||(count&&depth>=8)){search->failed=1;if(children)XFree(children);return;}
    search->remaining-=count;
    for(unsigned i=0;i<count&&!search->failed;i++) {
        unsigned w,h;unsigned long insets[4];
        if(window_pid(search->capture->display,children[i])==search->capture->owner) {
            if(!dimensions(search->capture->display,children[i],&w,&h)||!frame_insets(search->capture->display,children[i],insets)){search->failed=1;break;}
            if(search->owned++<8){size_t n=strlen(search->seen);snprintf(search->seen+n,sizeof(search->seen)-n,"%s%ux%u",n?",":"",w,h);}
            if((!search->selected||children[i]==search->selected)&&(search->window_height?
                w==(unsigned)search->window_width&&h==(unsigned)search->window_height:
                w==(unsigned)search->capture->width+insets[0]+insets[1]&&h>=(unsigned)search->capture->height+insets[2]+insets[3]&&h-search->capture->height<=400)) {
                search->capture->window=children[i];search->capture->window_width=w;search->capture->window_height=h;
                memcpy(search->capture->insets,insets,sizeof(insets));search->found++;
                continue; /* Its nested controls are part of this same client. */
            }
        }
        find_client_windows(search,children[i],depth+1);
    }
    if(children)XFree(children);
}
static struct Capture *capture_open(unsigned long owner,Window selected,int width,int height,
    int window_width,int window_height,int left,int top,int automatic) {
    if(owner==1||width<1||width>2560||height<1||height>1600)return NULL;
    if(window_height&&(!owner||window_width<1||window_width>2560||window_height<1||window_height>2400||
        left<0||top<0||left>window_width-width||top>window_height-height))return NULL;
    struct Capture *c = calloc(1,sizeof(*c));
    if (!c) return NULL;
    c->shm.shmid = -1; c->owner=owner; c->width=width; c->height=height;
    c->display=XOpenDisplay(NULL);
    int error;
    if (!c->display || !XShmQueryExtension(c->display) || !XDamageQueryExtension(c->display,&c->event,&error)) {open_refused("x_extensions");goto fail;}
    struct ClientSearch search={.capture=c,.selected=selected,.window_width=window_width,.window_height=window_height,.remaining=2048};
    if (!owner) {
        /* MP-08/MP-11: the whole owned desktop; its root is never redirected. */
        unsigned w,h;
        c->desktop=1;c->window=DefaultRootWindow(c->display);
        if (!dimensions(c->display,c->window,&w,&h) || w!=(unsigned)width || h!=(unsigned)height) {open_refused("desktop_geometry");goto fail;}
        c->window_width=width;c->window_height=height;c->pixmap=c->window;
        goto image;
    }
    find_client_windows(&search,DefaultRootWindow(c->display),0);
    if(search.failed){open_refused("window_tree_bound");goto fail;}
    if(search.found!=1) {
        fprintf(stderr,"MD-DISPLAY: native geometry requested %dx%d owned %s\n",width,height,search.owned?search.seen:"none");
        open_refused(search.found?"window_ambiguous":"window_not_found");goto fail;
    }
    c->xoffset=window_height?left:(int)c->insets[0];c->offset=window_height?top:c->window_height-(int)c->insets[3]-height;
    if(!bound_surface(c)){open_refused("window_retired");goto fail;}
    automatic=automatic||(getenv("CHARIOX_OWNED_VIRTUAL_DISPLAY")&&!strcmp(getenv("CHARIOX_OWNED_VIRTUAL_DISPLAY"),"1"));
    /* MP-08/MP-10: the shared Computer desktop presents the root, so its
     * browser client must remain automatically composited while captured. */
    if(!(automatic?redirect_window(c,CompositeRedirectAutomatic):
        (redirect_window(c,CompositeRedirectManual)||redirect_window(c,CompositeRedirectAutomatic)))){open_refused("composite_redirect");goto fail;}
    c->pixmap=XCompositeNameWindowPixmap(c->display,c->window); XSync(c->display,False);
    if (!c->pixmap) {open_refused("composite_pixmap");goto fail;}
image:
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
    c->hash_cur=malloc(height*sizeof(uint64_t));c->hash_last=malloc(height*sizeof(uint64_t));
    if (!c->previous||!c->hash_cur||!c->hash_last) goto fail;
    c->damage=XDamageCreate(c->display,c->window,XDamageReportRawRectangles);
    c->pid_atom=XInternAtom(c->display,"_NET_WM_PID",False);c->insets_atom=XInternAtom(c->display,"_GTK_FRAME_EXTENTS",False);
    if(!c->state_atom)c->state_atom=XInternAtom(c->display,"_NET_WM_STATE",False);
    XSelectInput(c->display,c->window,c->desktop?StructureNotifyMask:StructureNotifyMask|PropertyChangeMask);
    c->surface_dirty=1; /* the first readback proves the binding after the events are armed */
    return c;
fail:
    cx_capture_close(c); return NULL;
}
struct Capture *cx_capture_open_mode(unsigned long owner,int width,int height,int automatic) {
    return capture_open(owner,0,width,height,0,0,0,0,automatic);
}
struct Capture *cx_capture_open(unsigned long owner, int width, int height) {
    return capture_open(owner,0,width,height,0,0,0,0,0);
}
/* MP-08/MP-10/MP-11: kernel-resolved owned-window crop. The exact window
 * dimensions, crop and PID remain private worker state, never client authority.
 * Page P3 rectangles and the native Chrome band use this same source adapter. */
struct Capture *cx_capture_open_region(unsigned long owner, int window_width,
    int window_height, int left, int top, int width, int height) {
    if(!window_height)return NULL;
    return capture_open(owner,0,width,height,window_width,window_height,left,top,0);
}
/* MP-08/MP-10/MP-11: private kernel-resolved XID, still checked against
 * the exact process/window/crop. No client may supply this selection. */
struct Capture *cx_capture_open_selected_region(unsigned long owner,Window selected,
    int window_width,int window_height,int left,int top,int width,int height){
    if(selected<=1||selected>0xffffffffUL||!window_height)return NULL;
    return capture_open(owner,selected,width,height,window_width,window_height,left,top,0);
}
/* Numeric-only inventory on the kernel-owned private X server. Never names
 * a desktop/root surface, title, URL, pixels or credentials. */
struct InventorySearch {Display *display;Window root,focus;unsigned long owner;int64_t *out;int limit,found,failed,select;unsigned remaining;};
static void inventory_windows(struct InventorySearch *s,Window parent,unsigned depth){
    Window root,ancestor,*children=NULL;unsigned count=0;
    if(!XQueryTree(s->display,parent,&root,&ancestor,&children,&count)){s->failed=1;return;}
    if(count>s->remaining||(count&&depth>=8)){s->failed=1;if(children)XFree(children);return;}
    s->remaining-=count;
    for(unsigned i=0;i<count&&!s->failed;i++){
        if(window_pid(s->display,children[i])==s->owner){
            XWindowAttributes a;Window transient=0,child;int x,y;unsigned long insets[4];
            /* Selected before the reads below, so a change after them reaches the event queue. */
            if(s->select)XSelectInput(s->display,children[i],StructureNotifyMask|PropertyChangeMask|FocusChangeMask);
            if(!XGetWindowAttributes(s->display,children[i],&a)||a.width<1||a.height<1||s->found==s->limit||
                !XTranslateCoordinates(s->display,children[i],s->root,0,0,&x,&y,&child)||
                !frame_insets(s->display,children[i],insets)){s->failed=1;break;}
            XGetTransientForHint(s->display,children[i],&transient);
            int64_t *v=s->out+s->found++*13;v[0]=children[i];v[1]=a.width;v[2]=a.height;v[3]=x;v[4]=y;
            v[5]=a.map_state==IsViewable;v[6]=a.override_redirect;v[7]=transient;v[8]=s->focus==children[i];
            for(int j=0;j<4;j++)v[9+j]=insets[j];
            continue; /* Nested controls belong to this client, not another top-level surface. */
        }
        inventory_windows(s,children[i],depth+1);
    }
    if(children)XFree(children);
}
static int inventory_read(Display *d,unsigned long owner,int64_t *out,int limit,int select){
    struct InventorySearch s={.display=d,.root=DefaultRootWindow(d),.owner=owner,.out=out,.limit=limit,.select=select,.remaining=2048};
    int revert;XGetInputFocus(d,&s.focus,&revert);
    inventory_windows(&s,s.root,0);return s.failed?-1:s.found;
}
int cx_capture_inventory(unsigned long owner,int64_t *out,int limit){
    if(owner<=1||limit<1||limit>32)return -1;
    Display *d=XOpenDisplay(NULL);if(!d)return -1;
    int found=inventory_read(d,owner,out,limit,0);XCloseDisplay(d);return found;
}
/* L1: a persistent inventory connection. The walk is the one above; this adds the events that can
 * change its result (root children, and the owner windows' structure, focus and the properties the
 * rows read), so a caller can walk when told instead of polling. A missed event only delays a
 * discovery: every consumer still proves its window with the exact fence before use. */
struct Inventory {Display *display;unsigned long owner;Atom pid,state,insets,transient;};
static Display *inventory_display;
static XErrorHandler inventory_previous;
static int inventory_failed(Display *display,XErrorEvent *event) {
    if(display==inventory_display)return 0; /* a selected window that was destroyed meanwhile */
    return inventory_previous?inventory_previous(display,event):0;
}
struct Inventory *cx_inventory_open(unsigned long owner){
    if(owner<=1)return NULL;
    Display *d=XOpenDisplay(NULL);if(!d)return NULL;
    struct Inventory *v=calloc(1,sizeof(*v));if(!v){XCloseDisplay(d);return NULL;}
    v->display=d;v->owner=owner;
    v->pid=XInternAtom(d,"_NET_WM_PID",False);v->state=XInternAtom(d,"_NET_WM_STATE",False);
    v->insets=XInternAtom(d,"_GTK_FRAME_EXTENTS",False);v->transient=XInternAtom(d,"WM_TRANSIENT_FOR",False);
    inventory_display=d;inventory_previous=XSetErrorHandler(inventory_failed);
    XSelectInput(d,DefaultRootWindow(d),SubstructureNotifyMask);XSync(d,False);
    return v;
}
int cx_inventory_read(struct Inventory *v,int64_t *out,int limit){
    if(limit<1||limit>32)return -1;
    return inventory_read(v->display,v->owner,out,limit,1);
}
/* Drains the queue; 1 when an event that can change the rows arrived. */
int cx_inventory_changed(struct Inventory *v){
    int changed=0;
    while(XPending(v->display)){
        XEvent e;XNextEvent(v->display,&e);
        if(e.type==PropertyNotify){Atom a=e.xproperty.atom;changed|=a==v->pid||a==v->state||a==v->insets||a==v->transient;}
        else changed=1;
    }
    return changed;
}
/* 1: a row may have changed; 2: the wake fd is readable; 0: timeout; -1: connection lost. Events a
 * walk already read into Xlib's queue are checked before sleeping, so they wake the caller too. */
int cx_inventory_wait(struct Inventory *v,int wake,int timeout_ms){
    for(;;){
        if(cx_inventory_changed(v))return 1;
        struct pollfd p[2]={{.fd=ConnectionNumber(v->display),.events=POLLIN},{.fd=wake,.events=POLLIN}};
        int n=poll(p,2,timeout_ms);
        if(n<0&&errno==EINTR)continue;
        if(n<0||p[0].revents&(POLLHUP|POLLERR|POLLNVAL))return -1;
        if(n==0)return 0;
        if(p[1].revents)return 2;
    }
}
void cx_inventory_close(struct Inventory *v){if(v){XCloseDisplay(v->display);free(v);}}
void cx_capture_cpu(struct Capture *c,double *out) {memcpy(out,c->cpu,sizeof(c->cpu));}

int cx_capture_fd(struct Capture *c) { return ConnectionNumber(c->display); }
/* MP-11: readback and input use the same immutable window binding. Input
 * can arrive before the next XDamage/readback after a resize. Once changed,
 * the binding stays retired even if Chrome restores the previous size. */
static int owned_window_current(struct Capture *c) {
    if(c->retired||!bound_surface(c)){c->retired=1;return 0;}
    return 1;
}
/* MP-11: ancestors can hide the window (map_state) without an event on it, so the fence also
 * watches the chain below the root. Re-walked at every full check; a deeper tree grants nothing. */
static int watch_ancestors(struct Capture *c) {
    Window root,parent=c->window,*children=NULL;unsigned count=0;c->ancestor_count=0;
    if(c->desktop)return 1;
    int complete=0;
    for(int depth=0;depth<16&&!complete;depth++){
        if(!XQueryTree(c->display,parent,&root,&parent,&children,&count))return 0;
        if(children)XFree(children);
        if(parent==None||parent==root)complete=1;else c->ancestors[c->ancestor_count++]=parent;
    }
    if(!complete)return 0;
    /* A window destroyed meanwhile makes the select fail: refuse instead of dying on the X error. */
    XSync(c->display,False);redirect_display=c->display;redirect_error=0;
    redirect_previous=XSetErrorHandler(redirect_failed);
    for(int i=0;i<c->ancestor_count;i++)XSelectInput(c->display,c->ancestors[i],StructureNotifyMask);
    XSync(c->display,False);XSetErrorHandler(redirect_previous);redirect_display=NULL;
    return !redirect_error;
}
static int surface_atom(struct Capture *c,Atom atom) {return atom==c->pid_atom||atom==c->state_atom||atom==c->insets_atom;}
/* Everything bound_surface reads: attributes and mapping of the window and its ancestors, and
 * the PID, state and frame-extents properties of the window. */
static int surface_event(struct Capture *c,XEvent *event) {
    Window at=event->xany.window;int watched=at==c->window;
    for(int i=0;i<c->ancestor_count;i++)watched|=at==c->ancestors[i];
    if(!watched)return 0;
    switch(event->type){
        case ConfigureNotify:case MapNotify:case UnmapNotify:case DestroyNotify:case ReparentNotify:case GravityNotify:return 1;
        case PropertyNotify:return at==c->window&&surface_atom(c,event->xproperty.atom);
    }
    return 0;
}
/* Same effect as cx_capture_damage on a binding change; the wake it owes is delivered there. */
static void note_surface_event(struct Capture *c,XEvent *event) {
    if(event->type==DestroyNotify&&event->xdestroywindow.window==c->window)c->retired=1;
    if(event->type==UnmapNotify&&event->xunmap.window==c->window)c->retired=1;
    if(event->type==ConfigureNotify&&event->xconfigure.window==c->window&&
        (event->xconfigure.width!=c->window_width||event->xconfigure.height!=c->window_height))c->retired=1;
    c->surface_dirty=c->woken=1;
}
static Bool surface_predicate(Display *d,XEvent *event,XPointer arg) {(void)d;return surface_event((struct Capture *)arg,event);}
static void take_surface_events(struct Capture *c) {
    XEvent event;
    while(XCheckIfEvent(c->display,&event,surface_predicate,(XPointer)c))note_surface_event(c,&event);
}
/* The binding is current while no event reported a change since the last full proof. Events the
 * server generated before a reply reach the client before that reply. */
static int surface_current(struct Capture *c) {
    take_surface_events(c);
    if(c->retired)return 0;
    if(c->surface_dirty){
        c->surface_dirty=0;
        if(!watch_ancestors(c)||!bound_surface(c)){c->retired=1;return 0;}
    }
    return 1;
}
int cx_capture_damage(struct Capture *c) {
    int dirty=c->woken;c->woken=0;
    while (XPending(c->display)) {
        XEvent event; XNextEvent(c->display,&event);
        if(surface_event(c,&event)){note_surface_event(c,&event);dirty=1;}
        if (event.type==c->event) {
            XRectangle r=((XDamageNotifyEvent *)&event)->area;
            if (r.x<c->xoffset+c->width && r.x+r.width>c->xoffset && r.y<c->offset+c->height && r.y+r.height>c->offset) dirty=1;
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
/* MP-08/MP-10: sampled row fingerprint (six 32-pixel windows across the
 * row); votes only, never certifies pixels. */
static void row_hashes(const uint8_t *raw,int width,int height,uint64_t *out){
    size_t stride=(size_t)width*4;
    for(int y=0;y<height;y++){uint64_t h=0x9E3779B97F4A7C15ull;
        int n=width<32?width:32;
        for(int k=0;k<6;k++){
            int x=width*(2*k+1)/12-n/2;if(x<0)x=0;if(x>width-n)x=width-n;
            const uint8_t *p=raw+y*stride+(size_t)x*4;
            for(int i=0;i<n*4;i+=8){uint64_t v=0;int bytes=n*4-i<8?n*4-i:8;memcpy(&v,p+i,bytes);h=(h^v)*0xff51afd7ed558ccdull;h^=h>>29;}
        }
        out[y]=h;}
}
/* Group equal-state cells into band runs merged across adjacent bands.
 * Residual runs may bridge `gap` other rows: re-encoding exact pixels is
 * always correct, and fewer rectangles keep the plan within its bound. */
/* Cells are one row by one band: 128 pixels, 256 at Retina density; wider
 * cells give fewer, better-compressed residual images (probe-measured). */
static int shift_band(int width){return width>=2560?128:64;}
static int shift_runs(struct Capture *c,unsigned char want,int (*out)[4],int limit,int split,int gap){
    int band=shift_band(c->width),bands=(c->width+band-1)/band,count=0;
    for(int b=0;b<bands;b++){int x=b*band,w=c->width-x<band?c->width-x:band;
        for(int y=0;y<c->height;){
            if(c->state[y][b]!=want){y++;continue;}
            /* Cells matching both ways (state 3) may extend a move run but
             * never start one; runs stop at cells that must stay unmoved. */
            int y0=y,end=y;
            while(y0>0&&want==1&&c->state[y0-1][b]==3)y0--;
            while(y<c->height&&(!split||y-y0<split)){
                if(c->state[y][b]==want||(want==1&&c->state[y][b]==3)){end=++y;continue;}
                int g=y;while(g<c->height&&g-end<gap&&c->state[g][b]!=want)g++;
                if(g<c->height&&g-end<gap&&c->state[g][b]==want&&(!split||g-y0<split))y=g;else break;
            }
            y=end;
            int merged=0;
            for(int i=count-1;i>=0&&!merged;i--){
                if(out[i][0]+out[i][2]!=x)continue;
                if(out[i][1]==y0&&out[i][3]==y-y0){out[i][2]+=w;merged=1;continue;}
                /* Residuals may take the union of neighbouring runs: the
                 * extra rows are encoded exactly as well. */
                int top=out[i][1]<y0?out[i][1]:y0,bottom=out[i][1]+out[i][3]>y?out[i][1]+out[i][3]:y;
                if(want==2&&abs(out[i][1]-y0)<=gap&&abs(out[i][1]+out[i][3]-y)<=gap&&(!split||bottom-top<=split)){
                    out[i][1]=top;out[i][3]=bottom-top;out[i][2]+=w;merged=1;}
            }
            if(!merged){if(count==limit)return -1;out[count][0]=x;out[count][1]=y0;out[count][2]=w;out[count][3]=y-y0;count++;}
        }
    }
    return count;
}
/* Prove every cell for offset `dy` (0 = static) and group the rectangles. */
static int shift_decompose(struct Capture *c,const uint8_t *raw,const uint8_t *base,int dy,struct Shift *out){
    int width=c->width,height=c->height;size_t stride=(size_t)width*4;out->valid=0;
    int band=shift_band(width),bands=(width+band-1)/band,dirty_pixels=0,moved=0;
    /* First pass prefers moves and compares in place only when a move fails.
     * Static sidebars can then fragment moves; the second pass marks cells
     * matching both ways (state 3) so they join runs without starting them. */
    for(int both=0;both<2;both++){dirty_pixels=moved=0;
        for(int y=0;y<height;y++){const uint8_t *r=raw+y*stride,*same=base+y*stride,*from=y-dy>=0&&y-dy<height?base+(y-dy)*stride:NULL;
            for(int b=0;b<bands;b++){int x=b*band*4,n=(width-b*band<band?width-b*band:band)*4;
                int m=dy&&from&&!memcmp(r+x,from+x,n),u=(!m||both)&&!memcmp(r+x,same+x,n);
                unsigned char s=m&&u?3:m?1:u?0:2;
                c->state[y][b]=s;if(s==2)dirty_pixels+=n/4;else if(s!=0)moved+=n/4;}}
        if(dy){if(moved<width*height/4)return 0;
            out->moves=shift_runs(c,1,out->move,64,0,0);
            if(out->moves>=1)break;
        }else{if(dirty_pixels>width*height/4)return 0;out->moves=0;break;}
    }
    /* Wire tiles stay within the viewer's 2560x256 pixel bound. */
    int split=width>1280?width==2560?256:341:512;out->dirty=-1;
    for(int gap=4;gap<=256&&out->dirty<0;gap*=4)out->dirty=shift_runs(c,2,out->dirt,64,split,gap);
    /* Merge residual pairs whose union adds fewer exact pixels than a tile
     * header costs on the wire (~600 px at ~0.13 B/px). */
    for(int merged=1;merged&&out->dirty>1;){merged=0;
        for(int i=0;i<out->dirty&&!merged;i++)for(int j=i+1;j<out->dirty&&!merged;j++){
            int *a=out->dirt[i],*b=out->dirt[j],l=a[0]<b[0]?a[0]:b[0],t=a[1]<b[1]?a[1]:b[1];
            int r=a[0]+a[2]>b[0]+b[2]?a[0]+a[2]:b[0]+b[2],bt=a[1]+a[3]>b[1]+b[3]?a[1]+a[3]:b[1]+b[3];
            long extra=(long)(r-l)*(bt-t)-(long)a[2]*a[3]-(long)b[2]*b[3];
            if(extra<600&&(long)(r-l)*(bt-t)<=2560L*256&&bt-t<=split){
                a[0]=l;a[1]=t;a[2]=r-l;a[3]=bt-t;memmove(b,b+4,(size_t)(out->dirty-j-1)*4*sizeof(int));out->dirty--;merged=1;}
        }}
    if(out->dirty>=0){dirty_pixels=0;for(int i=0;i<out->dirty;i++)dirty_pixels+=out->dirt[i][2]*out->dirt[i][3];}
    if((dy&&out->moves<1)||out->dirty<0||(!dy&&out->dirty<1))return 0;
    out->dy=dy;out->dirty_pixels=dirty_pixels;out->valid=1;return 1;
}
static void shift_plan(struct Capture *c,const uint8_t *raw,const uint8_t *base,const uint64_t *base_hash,struct Shift *out,int static_ok){
    int height=c->height;out->valid=0;
    /* Each fingerprint keeps up to eight base rows (repeated text still votes);
     * blank rows repeat more often and never vote. */
    enum{SLOTS=4096,CHAIN=8};uint64_t keys[SLOTS];int head[SLOTS],count[SLOTS],next[1600];
    for(int i=0;i<SLOTS;i++)head[i]=-1;
    for(int y=0;y<height;y++){uint64_t k=base_hash[y];unsigned i=(unsigned)(k>>52)&(SLOTS-1);
        while(head[i]!=-1&&keys[i]!=k)i=(i+1)&(SLOTS-1);
        if(head[i]==-1){keys[i]=k;count[i]=0;}
        next[y]=head[i];head[i]=y;count[i]++;}
    /* MP-11: independent captures and test threads never share vote state. */
    int votes[3201]={0};
    int same=0;
    for(int y=0;y<height;y++){uint64_t k=c->hash_cur[y];unsigned i=(unsigned)(k>>52)&(SLOTS-1);
        while(head[i]!=-1&&keys[i]!=k)i=(i+1)&(SLOTS-1);
        if(head[i]<0||count[i]>CHAIN)continue;
        /* Distinctive rows only: blank rows match in place and everywhere. */
        for(int b=head[i];b>=0;b=next[b])if(b!=y)votes[y-b+1600]++;else same++;}
    /* Ties (periodic content) prefer the previous plan's offset. */
    int dy=0,best=0;for(int d=-height+1;d<height;d++){int v=votes[d+1600];
        if(d&&(v>best||(v==best&&v&&abs(d-c->last_dy)<abs(dy-c->last_dy)))){best=v;dy=d;}}
    /* Without a scroll (or when rows mostly stay in place) a static plan
     * re-encodes only changed cells; only the committed-canvas planner asks
     * for one, and it also covers a failed scroll candidate. */
    if(best>=8&&!(static_ok&&same>=best)&&shift_decompose(c,raw,base,dy,out)){c->last_dy=dy;return;}
    if(static_ok)shift_decompose(c,raw,base,0,out);
}
/* Return -1 on retirement, 0 for byte-identical, 1 for new immutable snapshot.
 * A sparse bound is proved from every row; XDamage bounds never authorize it. */
int cx_capture_read(struct Capture *c, uint8_t *out, int *bounds) {
    c->tile_count=c->adjacent_count=-1;c->shift.valid=0;
    memset(c->cpu,0,sizeof(c->cpu));double at=capture_cpu();
    if (!surface_current(c) || !XShmGetImage(c->display,c->pixmap,c->image,c->xoffset,c->offset,AllPlanes)) return -1;
    /* A change reported before the image reply makes this image unproven: prove it again or refuse. */
    if (!surface_current(c)) return -1;
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
    /* MP-08/MP-10: dense changes may be a vertical scroll of the previous readback. */
    c->shift.valid=0;
    if(c->adjacent_count<0&&c->plans){
        row_hashes(raw,c->width,c->height,c->hash_cur);
        if(c->last&&!c->hash_last_valid)row_hashes(c->last,c->width,c->height,c->hash_last);
        if(c->last)shift_plan(c,raw,c->last,c->hash_last,&c->shift,0);
        uint64_t *t=c->hash_last;c->hash_last=c->hash_cur;c->hash_cur=t;c->hash_last_valid=1;
    }else c->hash_last_valid=0;
    c->cpu[1]=capture_cpu()-at;at=capture_cpu();
    memcpy(out,raw,size);c->last=out;c->cpu[2]=capture_cpu()-at;
    return 1;
}

/* MP-08/MP-10/MP-11: exact sparse damage, not the union's collateral pixels. */
int cx_capture_tiles(struct Capture *c,int *out){if(c->tile_count>0)memcpy(out,c->tiles,c->tile_count*4*sizeof(int));return c->tile_count;}
int cx_capture_adjacent_tiles(struct Capture *c,int *out){if(c->adjacent_count>0)memcpy(out,c->adjacent,c->adjacent_count*4*sizeof(int));return c->adjacent_count;}
void cx_capture_admit(struct Capture *c,const uint8_t *pixels){
    size_t size=(size_t)c->width*c->height*4;
    int current=c->last&&(c->last==pixels||!memcmp(c->last,pixels,size));
    memcpy(c->previous,pixels,size);c->initialized=1;
    /* MP-11: an older/different admission retires the latest-byte shortcut.
     * A following read then conservatively compares as a fresh full capture. */
    c->last=current?c->previous:NULL;
}

/* MP-08/MP-10/MP-11: scheduling only; exact/base bounds stay conservative. */
int cx_capture_motion_height(struct Capture *c){return c->motion_height;}

/* MP-08/MP-10: [dy,moves,dirty,dirty_pixels, moves*4, dirty*4]; 0 if none. */
int cx_capture_shift(struct Capture *c,int *out){
    struct Shift *s=&c->shift;if(!s->valid)return 0;
    out[0]=s->dy;out[1]=s->moves;out[2]=s->dirty;out[3]=s->dirty_pixels;
    memcpy(out+4,s->move,s->moves*4*sizeof(int));memcpy(out+4+s->moves*4,s->dirt,s->dirty*4*sizeof(int));
    return 1;
}

/* MP-08/MP-10: the same proved decomposition against a retained exact canvas,
 * including a static plan (worker overlay fallback and tests). */
int cx_shift_plan(const uint8_t *raw,const uint8_t *base,int width,int height,int *out){
    if(width<1||width>2560||height<1||height>1600)return -1;
    struct Capture *c=calloc(1,sizeof(*c));if(!c)return -1;
    c->width=width;c->height=height;c->hash_cur=malloc(height*sizeof(uint64_t));uint64_t *hashes=malloc(height*sizeof(uint64_t));
    int result=-1;
    if(c->hash_cur&&hashes){row_hashes(raw,width,height,c->hash_cur);row_hashes(base,width,height,hashes);shift_plan(c,raw,base,hashes,&c->shift,1);result=cx_capture_shift(c,out);}
    free(hashes);free(c->hash_cur);free(c);return result;
}

/* MP-11 (review #893 P2): the pointer may reach only the owned window.
 * Chromium shows permission/download bubbles, autofill and <select> popups
 * as separate top-level windows outside the captured pixmap, so a viewer
 * never sees them. Resolves the owned window's root point (0: owned on top,
 * 1: another top-level window covers it, -1: not owned/unknown). */
static int pointer_target(struct Capture *c,int x,int y,int *px,int *py){
    if(x<0||y<0||x>=c->width||y>=c->height)return -1;
    if(!owned_window_current(c))return -1;
    Window root=DefaultRootWindow(c->display),child;int rx,ry,ox,oy;
    if(!XTranslateCoordinates(c->display,c->window,root,0,0,&rx,&ry,&child))return -1;
    *px=rx+c->xoffset+x;*py=ry+c->offset+y;
    // A WM frame reparents the client; follow only the topmost child at the
    // proved root point. Foreign overlays never grant the underlying client.
    Window parent=root;
    for(int depth=0;depth<8;depth++){
        if(!XTranslateCoordinates(c->display,root,parent,*px,*py,&ox,&oy,&child))return -1;
        if(child==c->window)return 0;
        if(!child)return 1;
        parent=child;
    }
    return -1;
}
/* MP-08/MP-10: wheel notches into the owned window on the kernel's private X
 * server, so Chromium applies native smooth scrolling (CDP wheel deltas are
 * precise and unanimated). Callers fence the document and actor first. */
int cx_capture_wheel(struct Capture *c,int x,int y,int dx,int dy){
    if(c->desktop)return -1; /* MP-11: desktop input keeps its own admission. */
    if(dx<-10||dx>10||dy<-10||dy>10||(!dx&&!dy))return -1;
    int px,py,target=pointer_target(c,x,y,&px,&py);
    if(target)return target;
    if(!XTestFakeMotionEvent(c->display,-1,px,py,CurrentTime))return -1;
    for(int i=0;i<abs(dy);i++){unsigned b=dy>0?5:4;XTestFakeButtonEvent(c->display,b,True,CurrentTime);XTestFakeButtonEvent(c->display,b,False,CurrentTime);}
    for(int i=0;i<abs(dx);i++){unsigned b=dx>0?7:6;XTestFakeButtonEvent(c->display,b,True,CurrentTime);XTestFakeButtonEvent(c->display,b,False,CurrentTime);}
    XFlush(c->display);return 0;
}

/* MP-08/MP-10: a click into the owned window (no renderer acknowledgement
 * round trip); the caller fenced document and actor first. Protocol 530:
 * button 2 (middle) and 3 (right) where the caller admitted them. */
int cx_capture_button(struct Capture *c,int x,int y,int button){
    if(c->desktop||button<1||button>3)return -1; /* MP-11: desktop input keeps its own admission. */
    int px,py,target=pointer_target(c,x,y,&px,&py);
    if(target)return target;
    if(!XTestFakeMotionEvent(c->display,-1,px,py,CurrentTime))return -1;
    XTestFakeButtonEvent(c->display,button,True,CurrentTime);XTestFakeButtonEvent(c->display,button,False,CurrentTime);
    XFlush(c->display);return 0;
}
int cx_capture_click(struct Capture *c,int x,int y){return cx_capture_button(c,x,y,1);}

/* Protocol 530: a primary-button drag between two points of the owned window
 * (the band's Tab reorder). Both ends must reach the owned window; the motion
 * is stepped (~130 ms) so Chrome's drag threshold and tab animation follow. */
int cx_capture_drag(struct Capture *c,int x,int y,int tx,int ty){
    if(c->desktop)return -1;
    int px,py,qx,qy,target=pointer_target(c,x,y,&px,&py);
    if(target)return target;
    if((target=pointer_target(c,tx,ty,&qx,&qy)))return target;
    if(!XTestFakeMotionEvent(c->display,-1,px,py,CurrentTime))return -1;
    XTestFakeButtonEvent(c->display,1,True,CurrentTime);XFlush(c->display);
    for(int i=1;i<=8;i++){
        struct timespec step={0,16*1000*1000};nanosleep(&step,NULL);
        XTestFakeMotionEvent(c->display,-1,px+(qx-px)*i/8,py+(qy-py)*i/8,CurrentTime);XFlush(c->display);
    }
    struct timespec settle={0,16*1000*1000};nanosleep(&settle,NULL);
    XTestFakeButtonEvent(c->display,1,False,CurrentTime);
    XFlush(c->display);return 0;
}

/* MP-08/MP-10: one key press/release into the owned window, focused first
 * (no renderer acknowledgement round trip); the caller fenced document,
 * actor and text target. A keysym outside the private keymap is refused. */
int cx_capture_key_modifiers(struct Capture *c,unsigned long keysym,int modifiers){
    if(c->desktop||modifiers<0||modifiers>15||!owned_window_current(c))return -1;
    KeyCode code=XKeysymToKeycode(c->display,(KeySym)keysym),shifter=XKeysymToKeycode(c->display,XK_Shift_L);
    if(!code||!shifter)return -1;
    if(XkbKeycodeToKeysym(c->display,code,0,0)!=(KeySym)keysym){
        if(XkbKeycodeToKeysym(c->display,code,0,1)!=(KeySym)keysym)return -1;
        modifiers|=8;
    }
    Window focus;int revert;XGetInputFocus(c->display,&focus,&revert);
    if(focus!=c->window){
        XSync(c->display,False);redirect_display=c->display;redirect_error=0;
        redirect_previous=XSetErrorHandler(redirect_failed);
        XSetInputFocus(c->display,c->window,RevertToParent,CurrentTime);XSync(c->display,False);
        XSetErrorHandler(redirect_previous);redirect_display=NULL;
        if(redirect_error)return -1;
    }
    KeySym names[4]={XK_Alt_L,XK_Control_L,XK_Super_L,XK_Shift_L};KeyCode mods[4]={0};
    for(int i=0;i<4;i++)if(modifiers&(1<<i)){mods[i]=XKeysymToKeycode(c->display,names[i]);if(!mods[i])return -1;}
    for(int i=0;i<4;i++)if(mods[i])XTestFakeKeyEvent(c->display,mods[i],True,CurrentTime);
    XTestFakeKeyEvent(c->display,code,True,CurrentTime);XTestFakeKeyEvent(c->display,code,False,CurrentTime);
    for(int i=3;i>=0;i--)if(mods[i])XTestFakeKeyEvent(c->display,mods[i],False,CurrentTime);
    XFlush(c->display);return 0;
}
int cx_capture_key(struct Capture *c,unsigned long keysym,int shift){
    return cx_capture_key_modifiers(c,keysym,shift?8:0);
}

/* MP-08/MP-10: plans cost a full compare; skip them while no viewer can use one. */
void cx_capture_plans(struct Capture *c,int enabled){c->plans=enabled!=0;if(!c->plans)c->hash_last_valid=0;}
