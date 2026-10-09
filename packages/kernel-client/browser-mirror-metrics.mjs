// MP-10: source-only 1-native-pixel edge band. Client holes cannot exempt themselves.
export function mirrorRasterMetrics(source,client) {
  if(source.width!==client.width||source.height!==client.height)throw Error('MP-10: raster geometry mismatch');
  const {width,height}=source,count=width*height,edges=new Uint8Array(count),band=new Uint8Array(count);
  const different=(a,b)=>[0,1,2].some(c=>source.data[a*4+c]!==source.data[b*4+c]);
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){const i=y*width+x;if(x+1<width&&different(i,i+1))edges[i]=edges[i+1]=1;if(y+1<height&&different(i,i+width))edges[i]=edges[i+width]=1;}
  for(let y=0;y<height;y++)for(let x=0;x<width;x++)if(edges[y*width+x])for(let dy=-1;dy<=1;dy++)for(let dx=-1;dx<=1;dx++)if(x+dx>=0&&x+dx<width&&y+dy>=0&&y+dy<height)band[(y+dy)*width+x+dx]=1;
  let bad=0,sum=0,outsideBad=0,outsideSum=0,outside=0;
  for(let i=0;i<count;i++){let changed=false,squared=0;for(let c=0;c<3;c++){const d=source.data[i*4+c]-client.data[i*4+c];changed||=d!==0;squared+=d*d;}bad+=+changed;sum+=squared;if(!band[i]){outside++;outsideBad+=+changed;outsideSum+=squared;}}
  return {pixel_mse:sum/(count*3),pixel_mismatch_fraction:bad/count,edge_band_pixels:count-outside,outside_edge_pixels:outside,outside_edge_mismatch_pixels:outsideBad,outside_edge_mismatch_fraction:outsideBad/outside,outside_edge_mse:outsideSum/(outside*3),band_definition:'source RGB transitions + 1 native pixel dilation, denominator = nonexcluded pixels'};
}
