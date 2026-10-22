#!/usr/bin/env python3
"""MP-08/MP-11: real bundle ranges behind the existing spike HTTP broker.

No guest sockets. This parent-side origin exposes only indexed bundle names.
The proxy does not read native caches: every GET goes to the pinned tar URL.
"""
import argparse,http.server,json,pathlib,threading,time,urllib.request,urllib.parse,os

def main():
 p=argparse.ArgumentParser();p.add_argument('--index',required=True);p.add_argument('--port-file',required=True);p.add_argument('--log',required=True);p.add_argument('--url',default='https://relay.fullyjustified.net/default_bundle_v33.tar');a=p.parse_args()
 index={}
 for line in pathlib.Path(a.index).read_text().splitlines():
  name,offset,size=line.split();index[name]=(int(offset),int(size))
 log=open(a.log,'a',buffering=1);lock=threading.Lock()
 class Handler(http.server.BaseHTTPRequestHandler):
  def log_message(self,*args):pass
  def do_GET(self):
   name=urllib.parse.unquote(self.path.removeprefix('/'))
   if name not in index:self.send_error(404);return
   offset,size=index[name];start=time.monotonic()
   try:
    req=urllib.request.Request(a.url,headers={'Range':f'bytes={offset}-{offset+size-1}','Accept-Encoding':'identity','User-Agent':'Tectonic/0.17.0'})
    with urllib.request.urlopen(req,timeout=60) as response:
     if response.status!=206:raise ValueError(f'upstream range status {response.status}')
     data=response.read(size+1)
    if len(data)!=size:raise ValueError(f'length {len(data)}, expected {size}')
    self.send_response(200);self.send_header('Content-Length',str(size));self.end_headers();self.wfile.write(data)
    row={'file':name,'bytes':size,'url':a.url,'range':[offset,offset+size-1],'ms':(time.monotonic()-start)*1000,'ok':True}
   except Exception as e:
    self.send_error(502);row={'file':name,'error':str(e),'ok':False}
   with lock:log.write(json.dumps(row)+'\n')
 server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
 pathlib.Path(a.port_file).write_text(str(server.server_port))
 pathlib.Path(a.port_file+'.pid').write_text(str(os.getpid()))
 server.serve_forever()
if __name__=='__main__':main()
