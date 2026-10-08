// MP-08/MP-10/MP-11: use the host VAAPI stack with its host loader first.
import {execFileSync} from 'node:child_process';
import {existsSync} from 'node:fs';
export function nativeLoader(executable, bundledLoader, bundledLibraries, probe = execFileSync, exists = existsSync) {
 const libraries='/usr/lib:/usr/lib/x86_64-linux-gnu:/lib:/lib/x86_64-linux-gnu:'+bundledLibraries;
 const failures=[];
 for(const loader of ['/lib64/ld-linux-x86-64.so.2','/lib/ld-linux-x86-64.so.2']){
  if(!exists(loader))continue;
  try{probe(loader,['--library-path',libraries,'--list',executable],{encoding:'utf8',timeout:10000,stdio:['ignore','pipe','pipe']});return {loader,library_path:libraries,mode:'host-vaapi-stack',failures};}
  catch(error){failures.push({loader,error:String(error.stderr||error.message).slice(0,4096)});}
 }
 return {loader:bundledLoader,library_path:bundledLibraries,mode:'bundled-fallback',failures};
}
