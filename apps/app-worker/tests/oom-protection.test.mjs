import assert from 'node:assert/strict';
import {test} from 'node:test';
import {spawnSync} from 'node:child_process';
import {mkdtemp,writeFile,readFile,rm,mkdir} from 'node:fs/promises';
import {homedir} from 'node:os';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

for (const policy of ['protected', 'positive']) {
 test(`the native App entry preserves a killable ${policy} OOM policy before admission`, {skip:process.platform!=='linux'},async(t)=>{
  if(policy==='protected'){
   const status=await readFile('/proc/self/status','utf8');
   const effective=BigInt(`0x${/^CapEff:\s*([0-9a-f]+)$/m.exec(status)[1]}`);
   if((effective & (1n << 24n))===0n){
    t.skip('privileged drill requires CAP_SYS_RESOURCE; run as capable root (docs/PROCESS_RESOURCES.md)');
    return;
   }
  }
  const parent=path.join(homedir(),'.chariox/dev/app-oom-regression');await mkdir(parent,{recursive:true});
  const scratch=await mkdtemp(path.join(parent,'run-'));
  try{
   const source=fileURLToPath(new URL('../src/linux_domain_entry.c',import.meta.url));
   const probe=path.join(scratch,'probe.c'),binary=path.join(scratch,'probe');
   await writeFile(probe,`#define main domain_main
#include ${JSON.stringify(source)}
#undef main
int main(void) {
 const char *inherited=${JSON.stringify(policy==='protected'?'-1000':'350')};
 const char *expected=${JSON.stringify(policy==='protected'?'0\n':'350\n')};
 int fd=open("/proc/self/oom_score_adj",O_WRONLY|O_CLOEXEC);
 if(fd<0||write(fd,inherited,strlen(inherited))!=(ssize_t)strlen(inherited)||close(fd))return 70;
 /* Establish a privileged positive minimum, then lose CAP_SYS_RESOURCE. */
 if(inherited[0]!='-' && geteuid()==0 && (setgroups(0,NULL)||setgid(65534)||setuid(65534)))return 74;
 if(prctl(PR_SET_DUMPABLE,1,0,0,0))return 75;
 /* Invalid descriptors refuse admission, after applying the OOM policy. */
 if(domain_main(0,NULL)!=DOMAIN_FAILURE)return 71;
 if(prctl(PR_GET_DUMPABLE,0,0,0,0)!=0)return 76;
 char score[16]={0}; fd=open("/proc/self/oom_score_adj",O_RDONLY|O_CLOEXEC);
 if(fd<0||read(fd,score,15)<=0||close(fd))return 72;
 return strcmp(score,expected)?73:0;
}
`);
   const compile=spawnSync('/usr/bin/cc',['-Wall','-Wextra','-Werror',probe,'-o',binary],{encoding:'utf8',timeout:60000});assert.equal(compile.status,0,compile.stderr);
   const result=spawnSync(binary,[],{encoding:'utf8',timeout:10000});assert.equal(result.status,0,`App entry must apply ${policy} policy (exit ${result.status})`);
  }finally{await rm(scratch,{recursive:true,force:true})}
 });
}
