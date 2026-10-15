/* MP-08/MP-10/MP-11: selected crops and inventory retain #958's WM client binding. */
#include <assert.h>
#include <unistd.h>
#include <stdio.h>
#include CAPTURE_SOURCE
int main(void){
 Display*d=XOpenDisplay(NULL);assert(d);Window root=DefaultRootWindow(d);
 Window frame=XCreateSimpleWindow(d,root,10,10,160,160,0,0,0);
 Window client=XCreateSimpleWindow(d,frame,4,4,128,128,0,0,0x112233);
 unsigned long pid=getpid();XChangeProperty(d,client,XInternAtom(d,"_NET_WM_PID",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)&pid,1);
 unsigned long insets[4]={4,4,0,4};XChangeProperty(d,client,XInternAtom(d,"_GTK_FRAME_EXTENTS",False),XA_CARDINAL,32,PropModeReplace,(unsigned char*)insets,4);
 XMapWindow(d,client);XMapWindow(d,frame);XSync(d,False);
 int64_t rows[32*13];int count=cx_capture_inventory(pid,rows,32);
 assert(count==1&&rows[0]==(int64_t)client&&rows[1]==128&&rows[2]==128);
 assert(rows[9]==4&&rows[10]==4&&rows[11]==0&&rows[12]==4);
 struct Capture*c=cx_capture_open_selected_region(pid,client,128,128,16,16,32,32);assert(c);
 uint8_t pixels[32*32*4];int bounds[4];assert(cx_capture_read(c,pixels,bounds)==1);
 assert(pixels[0]==0x33&&pixels[1]==0x22&&pixels[2]==0x11);
 assert(cx_capture_open_selected_region(pid,frame,160,160,16,16,32,32)==NULL);
 int x,y;assert(pointer_target(c,2,3,&x,&y)==0&&x==32&&y==33);
 XResizeWindow(d,client,130,128);XSync(d,False);assert(cx_capture_read(c,pixels,bounds)==-1);
 XResizeWindow(d,client,128,128);XSync(d,False);assert(cx_capture_read(c,pixels,bounds)==-1);
 cx_capture_close(c);XDestroyWindow(d,frame);XCloseDisplay(d);
 puts("MP-08/MP-10/MP-11 reparented client inventory/crop/retirement PASS");
}
