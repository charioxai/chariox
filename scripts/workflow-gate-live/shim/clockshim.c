// Test-only LD_PRELOAD: adds FAKE_CLOCK_OFFSET_S seconds to CLOCK_REALTIME reads.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <time.h>
#include <sys/time.h>
static long off(void){const char*e=getenv("FAKE_CLOCK_OFFSET_S");return e?atol(e):0;}
int clock_gettime(clockid_t c,struct timespec*t){
  static int(*real)(clockid_t,struct timespec*);
  if(!real)real=dlsym(RTLD_NEXT,"clock_gettime");
  int r=real(c,t);
  if(r==0&&(c==CLOCK_REALTIME||c==CLOCK_REALTIME_COARSE))t->tv_sec+=off();
  return r;}
int gettimeofday(struct timeval*tv,void*tz){
  static int(*real)(struct timeval*,void*);
  if(!real)real=dlsym(RTLD_NEXT,"gettimeofday");
  int r=real(tv,tz);
  if(r==0&&tv)tv->tv_sec+=off();
  return r;}
time_t time(time_t*out){struct timespec t;clock_gettime(CLOCK_REALTIME,&t);if(out)*out=t.tv_sec;return t.tv_sec;}
