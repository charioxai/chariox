/* MP-08/MP-10: lossless WebP for exact repair strips and scroll residuals.
 * Input is the owned BGRX raster; output is decoded RGB-exact by viewers. */
#include <stddef.h>
#include <stdint.h>
#include <webp/encode.h>

int cx_webp_lossless(const uint8_t *bgrx,int stride,int width,int height,int method,int quality,uint8_t **out,size_t *length) {
    WebPConfig config;WebPPicture picture;WebPMemoryWriter writer;
    *out=NULL;*length=0;
    if(width<1||height<1||width>2560||height>1600||stride<width*4||method<0||method>6||quality<0||quality>100)return -1;
    if(!WebPConfigInit(&config)||!WebPPictureInit(&picture))return -1;
    config.lossless=1;config.method=method;config.quality=(float)quality;config.exact=1;
    picture.use_argb=1;picture.width=width;picture.height=height;
    if(!WebPValidateConfig(&config)||!WebPPictureImportBGRX(&picture,bgrx,stride))return -1;
    WebPMemoryWriterInit(&writer);picture.writer=WebPMemoryWrite;picture.custom_ptr=&writer;
    int ok=WebPEncode(&config,&picture);WebPPictureFree(&picture);
    if(!ok||!writer.size){WebPMemoryWriterClear(&writer);return -1;}
    *out=writer.mem;*length=writer.size;return 0;
}
void cx_webp_free(uint8_t *bytes){WebPFree(bytes);}
