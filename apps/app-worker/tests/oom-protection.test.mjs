import assert from 'node:assert/strict';
import {test} from 'node:test';
import {spawnSync} from 'node:child_process';
import {mkdtemp,writeFile,rm,mkdir} from 'node:fs/promises';
import {homedir} from 'node:os';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

test('the native App entry drops inherited kernel OOM immunity before domain admission', {skip:process.platform!=='linux'},async()=>{
 const parent=path.join(homedir(),'.chariox/dev/app-oom-regression');await mkdir(parent,{recursive:true});
 const scratch=await mkdtemp(path.join(parent,'run-'));
 try{
  const source=fileURLToPath(new URL('../src/linux_domain_entry.c',import.meta.url));
  const probe=path.join(scratch,'probe.c'),binary=path.join(scratch,'probe');
  await writeFile(probe,`#define main domain_main\n#include ${JSON.stringify(source)}\n#undef main\nint main(void) {\nint fd=open("/proc/self/oom_score_adj",O_WRONLY|O_CLOEXEC);\nif(fd<0||write(fd,"-1000",5)!=5||close(fd))return 70;\n/* No cgroup descriptors: admission must fail, after resetting immunity. */\nif(domain_main(0,NULL)!=DOMAIN_FAILURE)return 71;\nchar score[16]={0}; fd=open("/proc/self/oom_score_adj",O_RDONLY|O_CLOEXEC);\nif(fd<0||read(fd,score,15)<=0||close(fd))return 72;\nreturn strcmp(score,"0\\n")?73:0;\n}\n`);
  const compile=spawnSync('/usr/bin/cc',['-Wall','-Wextra','-Werror',probe,'-o',binary],{encoding:'utf8',timeout:60000});assert.equal(compile.status,0,compile.stderr);
  const result=spawnSync(binary,[],{encoding:'utf8',timeout:10000});assert.equal(result.status,0,`App entry must remove inherited -1000 (exit ${result.status})`);
 }finally{await rm(scratch,{recursive:true,force:true})}
});
