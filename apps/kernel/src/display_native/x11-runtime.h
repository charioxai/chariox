/* MP-08/MP-10: load the display stack only when a native worker opens it.
 * Xlib/Xext keep their public ABI across supported distributions. */
#include <dlfcn.h>
#include <pthread.h>

#define CX_X11_FUNCTIONS(F) F(XCheckIfEvent) \
    F(XCloseDisplay) \
    F(XFlush) \
    F(XFree) \
    F(XFreePixmap) \
    F(XGetGeometry) \
    F(XGetInputFocus) \
    F(XGetTransientForHint) \
    F(XGetWindowAttributes) \
    F(XGetWindowProperty) \
    F(XInternAtom) \
    F(XKeysymToKeycode) \
    F(XNextEvent) \
    F(XOpenDisplay) \
    F(XPending) \
    F(XQueryTree) \
    F(XSelectInput) \
    F(XSetErrorHandler) \
    F(XSetInputFocus) \
    F(XSync) \
    F(XTranslateCoordinates) \
    F(XkbKeycodeToKeysym)
#define CX_XEXT_FUNCTIONS(F) F(XShmAttach) \
    F(XShmCreateImage) \
    F(XShmDetach) \
    F(XShmGetImage) \
    F(XShmQueryExtension)
#define CX_XDAMAGE_FUNCTIONS(F) F(XDamageCreate) \
    F(XDamageDestroy) \
    F(XDamageQueryExtension)
#define CX_XCOMPOSITE_FUNCTIONS(F) F(XCompositeNameWindowPixmap) \
    F(XCompositeRedirectWindow) \
    F(XCompositeUnredirectWindow)
#define CX_XTST_FUNCTIONS(F) F(XTestFakeButtonEvent) \
    F(XTestFakeKeyEvent) \
    F(XTestFakeMotionEvent)
#define CX_X_POINTER(name) static __typeof__(&name) cx_##name;
CX_X11_FUNCTIONS(CX_X_POINTER)
CX_XEXT_FUNCTIONS(CX_X_POINTER)
CX_XDAMAGE_FUNCTIONS(CX_X_POINTER)
CX_XCOMPOSITE_FUNCTIONS(CX_X_POINTER)
CX_XTST_FUNCTIONS(CX_X_POINTER)
#undef CX_X_POINTER
static pthread_once_t x11_once=PTHREAD_ONCE_INIT;
static int x11_ready;
static void x11_load(void) {
    void *libraries[5]={0};
    const char *missing=NULL;
    missing="libX11.so.6";
    libraries[0]=dlopen(missing,RTLD_NOW|RTLD_LOCAL);
    if(!libraries[0])goto unavailable;
#define CX_X_RESOLVE(name) if(!(*(void **)(&cx_##name)=dlsym(libraries[0],#name)))goto unavailable;
    CX_X11_FUNCTIONS(CX_X_RESOLVE)
#undef CX_X_RESOLVE
    missing="libXext.so.6";
    libraries[1]=dlopen(missing,RTLD_NOW|RTLD_LOCAL);
    if(!libraries[1])goto unavailable;
#define CX_X_RESOLVE(name) if(!(*(void **)(&cx_##name)=dlsym(libraries[1],#name)))goto unavailable;
    CX_XEXT_FUNCTIONS(CX_X_RESOLVE)
#undef CX_X_RESOLVE
    missing="libXdamage.so.1";
    libraries[2]=dlopen(missing,RTLD_NOW|RTLD_LOCAL);
    if(!libraries[2])goto unavailable;
#define CX_X_RESOLVE(name) if(!(*(void **)(&cx_##name)=dlsym(libraries[2],#name)))goto unavailable;
    CX_XDAMAGE_FUNCTIONS(CX_X_RESOLVE)
#undef CX_X_RESOLVE
    missing="libXcomposite.so.1";
    libraries[3]=dlopen(missing,RTLD_NOW|RTLD_LOCAL);
    if(!libraries[3])goto unavailable;
#define CX_X_RESOLVE(name) if(!(*(void **)(&cx_##name)=dlsym(libraries[3],#name)))goto unavailable;
    CX_XCOMPOSITE_FUNCTIONS(CX_X_RESOLVE)
#undef CX_X_RESOLVE
    missing="libXtst.so.6";
    libraries[4]=dlopen(missing,RTLD_NOW|RTLD_LOCAL);
    if(!libraries[4])goto unavailable;
#define CX_X_RESOLVE(name) if(!(*(void **)(&cx_##name)=dlsym(libraries[4],#name)))goto unavailable;
    CX_XTST_FUNCTIONS(CX_X_RESOLVE)
#undef CX_X_RESOLVE
    x11_ready=1;return;
unavailable:
    fprintf(stderr,"MD-DISPLAY: native display unavailable: %s missing or incompatible; install libx11-6 libxext6 libxdamage1 libxcomposite1 libxtst6\n",missing);
    for(int i=4;i>=0;i--)if(libraries[i])dlclose(libraries[i]);
}
static Display *open_display(const char *name) {
    pthread_once(&x11_once,x11_load);
    return x11_ready?cx_XOpenDisplay(name):NULL;
}
#define XCheckIfEvent (*cx_XCheckIfEvent)
#define XCloseDisplay (*cx_XCloseDisplay)
#define XFlush (*cx_XFlush)
#define XFree (*cx_XFree)
#define XFreePixmap (*cx_XFreePixmap)
#define XGetGeometry (*cx_XGetGeometry)
#define XGetInputFocus (*cx_XGetInputFocus)
#define XGetTransientForHint (*cx_XGetTransientForHint)
#define XGetWindowAttributes (*cx_XGetWindowAttributes)
#define XGetWindowProperty (*cx_XGetWindowProperty)
#define XInternAtom (*cx_XInternAtom)
#define XKeysymToKeycode (*cx_XKeysymToKeycode)
#define XNextEvent (*cx_XNextEvent)
#define XOpenDisplay open_display
#define XPending (*cx_XPending)
#define XQueryTree (*cx_XQueryTree)
#define XSelectInput (*cx_XSelectInput)
#define XSetErrorHandler (*cx_XSetErrorHandler)
#define XSetInputFocus (*cx_XSetInputFocus)
#define XSync (*cx_XSync)
#define XTranslateCoordinates (*cx_XTranslateCoordinates)
#define XkbKeycodeToKeysym (*cx_XkbKeycodeToKeysym)
#define XShmAttach (*cx_XShmAttach)
#define XShmCreateImage (*cx_XShmCreateImage)
#define XShmDetach (*cx_XShmDetach)
#define XShmGetImage (*cx_XShmGetImage)
#define XShmQueryExtension (*cx_XShmQueryExtension)
#define XDamageCreate (*cx_XDamageCreate)
#define XDamageDestroy (*cx_XDamageDestroy)
#define XDamageQueryExtension (*cx_XDamageQueryExtension)
#define XCompositeNameWindowPixmap (*cx_XCompositeNameWindowPixmap)
#define XCompositeRedirectWindow (*cx_XCompositeRedirectWindow)
#define XCompositeUnredirectWindow (*cx_XCompositeUnredirectWindow)
#define XTestFakeButtonEvent (*cx_XTestFakeButtonEvent)
#define XTestFakeKeyEvent (*cx_XTestFakeKeyEvent)
#define XTestFakeMotionEvent (*cx_XTestFakeMotionEvent)
