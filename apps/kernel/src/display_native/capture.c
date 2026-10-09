/* MP-08/MP-10/MP-11: owned-window XShm readback. Events wake, exact bytes
 * authorize damage reuse. No desktop/root pixels, no provider state. */
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
    int attached, event, width, height, window_height, offset, redirected, redirect_mode;
    unsigned long owner;
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
struct Capture *cx_capture_open(unsigned long owner, int width, int height) {
    struct Capture *c = calloc(1,sizeof(*c));
    if (!c) return NULL;
    c->shm.shmid = -1; c->owner=owner; c->width=width; c->height=height;
    c->display=XOpenDisplay(NULL);
    int error;
    if (!c->display || !XShmQueryExtension(c->display) || !XDamageQueryExtension(c->display,&c->event,&error)) {open_refused("x_extensions");goto fail;}
    Window root,parent,*children=NULL; unsigned count=0, found=0, owned=0;
    char seen[256]="";
    if (!XQueryTree(c->display,DefaultRootWindow(c->display),&root,&parent,&children,&count)) {open_refused("x_tree");goto fail;}
    for (unsigned i=0;i<count;i++) {
        unsigned w,h;
        if (window_pid(c->display,children[i])!=owner || !dimensions(c->display,children[i],&w,&h)) continue;
        if (owned++<8) {size_t n=strlen(seen);snprintf(seen+n,sizeof(seen)-n,"%s%ux%u",n?",":"",w,h);}
        if (w==(unsigned)width && h>=(unsigned)height && h-height<=400) {
            c->window=children[i]; c->window_height=h; found++;
        }
    }
    if (children) XFree(children);
    if (found!=1) {
        fprintf(stderr,"MD-DISPLAY: native geometry requested %dx%d owned %s\n",width,height,owned?seen:"none");
        open_refused(found?"window_ambiguous":"window_not_found");goto fail;
    }
    c->offset=c->window_height-height;
    /* The kernel owns a private X server; its root is never presented. Manual
     * keeps the named owned-window pixmap without compositing an unused root. */
    if(!redirect_window(c,CompositeRedirectManual)&&!redirect_window(c,CompositeRedirectAutomatic)){open_refused("composite_redirect");goto fail;}
    c->pixmap=XCompositeNameWindowPixmap(c->display,c->window); XSync(c->display,False);
    if (!c->pixmap) {open_refused("composite_pixmap");goto fail;}
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
/* MP-08/MP-10: sampled row fingerprint (six 32-pixel windows across the
 * row); votes only, never certifies pixels. */
static void row_hashes(const uint8_t *raw,int width,int height,uint64_t *out){
    size_t stride=(size_t)width*4;
    for(int y=0;y<height;y++){uint64_t h=0x9E3779B97F4A7C15ull;
        for(int k=0;k<6;k++){const uint8_t *p=raw+y*stride+(size_t)(width*(2*k+1)/12-16)*4;for(int i=0;i<128;i+=8){uint64_t v;memcpy(&v,p+i,8);h=(h^v)*0xff51afd7ed558ccdull;h^=h>>29;}}
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

/* MP-08/MP-10: wheel notches into the owned window on the kernel's private X
 * server, so Chromium applies native smooth scrolling (CDP wheel deltas are
 * precise and unanimated). Callers fence the document and actor first. */
int cx_capture_wheel(struct Capture *c,int x,int y,int dx,int dy){
    if(x<0||y<0||x>=c->width||y>=c->height||dx<-10||dx>10||dy<-10||dy>10||(!dx&&!dy))return -1;
    if(window_pid(c->display,c->window)!=c->owner)return -1;
    Window child;int rx,ry;
    if(!XTranslateCoordinates(c->display,c->window,DefaultRootWindow(c->display),0,0,&rx,&ry,&child))return -1;
    if(!XTestFakeMotionEvent(c->display,-1,rx+x,ry+c->offset+y,CurrentTime))return -1;
    for(int i=0;i<abs(dy);i++){unsigned b=dy>0?5:4;XTestFakeButtonEvent(c->display,b,True,CurrentTime);XTestFakeButtonEvent(c->display,b,False,CurrentTime);}
    for(int i=0;i<abs(dx);i++){unsigned b=dx>0?7:6;XTestFakeButtonEvent(c->display,b,True,CurrentTime);XTestFakeButtonEvent(c->display,b,False,CurrentTime);}
    XFlush(c->display);return 0;
}

/* MP-08/MP-10: a primary-button click into the owned window (no renderer
 * acknowledgement round trip); the caller fenced document and actor first. */
int cx_capture_click(struct Capture *c,int x,int y){
    if(x<0||y<0||x>=c->width||y>=c->height)return -1;
    if(window_pid(c->display,c->window)!=c->owner)return -1;
    Window child;int rx,ry;
    if(!XTranslateCoordinates(c->display,c->window,DefaultRootWindow(c->display),0,0,&rx,&ry,&child))return -1;
    if(!XTestFakeMotionEvent(c->display,-1,rx+x,ry+c->offset+y,CurrentTime))return -1;
    XTestFakeButtonEvent(c->display,1,True,CurrentTime);XTestFakeButtonEvent(c->display,1,False,CurrentTime);
    XFlush(c->display);return 0;
}

/* MP-08/MP-10: one key press/release into the owned window, focused first
 * (no renderer acknowledgement round trip); the caller fenced document,
 * actor and text target. A keysym outside the private keymap is refused. */
int cx_capture_key(struct Capture *c,unsigned long keysym,int shift){
    if(window_pid(c->display,c->window)!=c->owner)return -1;
    KeyCode code=XKeysymToKeycode(c->display,(KeySym)keysym),shifter=XKeysymToKeycode(c->display,XK_Shift_L);
    if(!code||!shifter)return -1;
    if(XkbKeycodeToKeysym(c->display,code,0,0)!=(KeySym)keysym){
        if(XkbKeycodeToKeysym(c->display,code,0,1)!=(KeySym)keysym)return -1;
        shift=1;
    }
    Window focus;int revert;XGetInputFocus(c->display,&focus,&revert);
    if(focus!=c->window){
        XSync(c->display,False);redirect_display=c->display;redirect_error=0;
        redirect_previous=XSetErrorHandler(redirect_failed);
        XSetInputFocus(c->display,c->window,RevertToParent,CurrentTime);XSync(c->display,False);
        XSetErrorHandler(redirect_previous);redirect_display=NULL;
        if(redirect_error)return -1;
    }
    if(shift)XTestFakeKeyEvent(c->display,shifter,True,CurrentTime);
    XTestFakeKeyEvent(c->display,code,True,CurrentTime);XTestFakeKeyEvent(c->display,code,False,CurrentTime);
    if(shift)XTestFakeKeyEvent(c->display,shifter,False,CurrentTime);
    XFlush(c->display);return 0;
}

/* MP-08/MP-10: plans cost a full compare; skip them while no viewer can use one. */
void cx_capture_plans(struct Capture *c,int enabled){c->plans=enabled!=0;if(!c->plans)c->hash_last_valid=0;}
