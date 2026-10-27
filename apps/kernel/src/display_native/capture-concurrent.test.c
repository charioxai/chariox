/* MP-08/MP-10/MP-11: concurrent planners must keep independent vote state.
 * Supplementary native regression; no browser/relay acceptance claim. */
#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
extern int cx_shift_plan(const uint8_t *,const uint8_t *,int,int,int *);
static pthread_barrier_t barrier;
static void *plan(void *arg){
    const int w=1280,h=800,dy=(int)(intptr_t)arg;
    uint8_t *base=malloc((size_t)w*h*4),*raw=malloc((size_t)w*h*4);
    assert(base&&raw);
    for(int y=0;y<h;y++)for(int x=0;x<w;x++){
        size_t at=((size_t)y*w+x)*4;
        for(int n=0;n<2;n++){
            int row=y-(n?dy:0);uint8_t *p=(n?raw:base)+at;
            p[0]=(uint8_t)((x*31+row*17)^row*row);p[1]=(uint8_t)(row*13);
            p[2]=(uint8_t)(x/5+row/256*77);p[3]=0;
        }
    }
    for(int i=0;i<100;i++){
        int out[4+128*4];pthread_barrier_wait(&barrier);
        int ok=cx_shift_plan(raw,base,w,h,out);
        if(ok!=1||out[0]!=dy){fprintf(stderr,"MP-11: planner expected %d got %d (result %d)\n",dy,out[0],ok);abort();}
    }
    free(base);free(raw);return NULL;
}
int main(void){
    const int offsets[]={-37,-53,17,29};pthread_t threads[4];
    assert(pthread_barrier_init(&barrier,NULL,4)==0);
    for(int i=0;i<4;i++)assert(pthread_create(&threads[i],NULL,plan,(void *)(intptr_t)offsets[i])==0);
    for(int i=0;i<4;i++)assert(pthread_join(threads[i],NULL)==0);
    pthread_barrier_destroy(&barrier);puts("MP-08/MP-10/MP-11: 400 concurrent native plans matched their own offset");return 0;
}
