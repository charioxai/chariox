// MP-10/MP-11: one isolated local-network fixture for every display adapter.
import {execFileSync} from 'node:child_process';
export const namespaceIp=(...args)=>execFileSync('/usr/sbin/ip',args,{encoding:'utf8'});
export function configureNamespace(name){
  if(!/^md-display-[0-9a-f]{12}$/.test(name))throw Error('MP-10 foreign namespace');
  namespaceIp('netns','exec',name,'ip','link','set','lo','mtu','1500','up');
  // WebRTC excludes a loopback-only host from normal ICE candidate gathering.
  // This unrouted TEST-NET interface exists only in the newly owned namespace.
  namespaceIp('netns','exec',name,'ip','link','add','md0','type','dummy');
  namespaceIp('netns','exec',name,'ip','addr','add','192.0.2.2/24','dev','md0');
  namespaceIp('netns','exec',name,'ip','link','set','md0','mtu','1500','up');
  execFileSync('/usr/sbin/ip',['netns','exec',name,'ethtool','-K','lo','tso','off','gso','off','gro','off'],{stdio:'ignore'});
}
