import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { closeSync, fstatSync, openSync } from "node:fs"
import { mkdtemp, rename, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { digestPinnedHomeArchive } from "../apps/kernel/slice-linux-docker/managed-home-archive-digest.mjs"

const sha256 = bytes => createHash("sha256").update(bytes).digest("hex")

test("MP-08 MP-10 MP-11 archive digest rejects invalid inactivity policies", async context => {
  const root = await mkdtemp(join(tmpdir(), "chariox-archive-policy-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const path = join(root, "archive")
  await writeFile(path, "synthetic", { mode: 0o600 })
  const fd = openSync(path, "r")
  context.after(() => closeSync(fd))
  for (const value of [0, -1, null, Infinity, 1.5, 2_147_483_648]) {
    await assert.rejects(digestPinnedHomeArchive(fd, value), /invalid home archive progress timeout/)
  }
})

test("managed digest retains the pinned inode and leaves its caller descriptor open", {
  skip: process.platform !== "linux" && "managed broker requires Linux proc descriptors",
}, async context => {
  const root = await mkdtemp(join(tmpdir(), "chariox-archive-digest-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const path = join(root, "archive")
  const contents = Buffer.alloc(160 * 1024, 173)
  await writeFile(path, contents, { mode: 0o600 })
  const fd = openSync(path, "r")
  context.after(() => closeSync(fd))
  const before = fstatSync(fd)
  await rename(path, join(root, "retained"))
  await writeFile(path, "replacement")
  assert.equal(await digestPinnedHomeArchive(fd, 1000), sha256(contents))
  const after = fstatSync(fd)
  assert.equal(after.ino, before.ino)
  assert.equal(after.size, contents.length)
  assert.equal(after.mode & 0o777, 0o600)
})

test("MP-08 MP-11 ordinary preverification uses the descriptor-owned progress supervisor", async () => {
 const {readFile}=await import("node:fs/promises")
 const source=await readFile(new URL("../apps/kernel/src/slice/local_docker/state.rs",import.meta.url),"utf8")
 const hash=source.slice(source.indexOf("fn file_sha256("),source.indexOf("fn valid_sha256_digest("))
 assert.doesNotMatch(hash,/std::io::copy/)
 assert.match(hash,/home_archive_verify::digest/)
})

for (const fault of ["before", "after", "healthy"]) test(`MP-08 MP-10 MP-11 pinned supervisor covers syscall progress: ${fault}`, {
 skip: process.platform !== "linux" && "requires Linux syscall injection", timeout: 6000,
}, async context => {
 const {spawnSync}=await import("node:child_process")
 const {readFile}=await import("node:fs/promises")
 const root=await mkdtemp(join(tmpdir(),"chariox-verify-syscall-"))
 context.after(()=>rm(root,{recursive:true,force:true}))
 const archive=join(root,"archive"),marker=join(root,"started"),source=join(root,"stall.c"),library=join(root,"stall.so")
 const contents=Buffer.alloc((fault === "healthy" ? 1024 : 256)*1024,111)
 await writeFile(archive,contents,{mode:0o600})
 const fd=openSync(archive,"r"), identity=fstatSync(fd,{bigint:true});closeSync(fd)
 await writeFile(source,`
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <stdio.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>
ssize_t read(int fd, void *buf, size_t count) {
 static ssize_t (*actual)(int,void*,size_t); static int reads;
 if(!actual) actual=dlsym(RTLD_NEXT,"read");
 struct stat s;
 if(fstat(fd,&s)==0 && s.st_dev==${identity.dev}ULL && s.st_ino==${identity.ino}ULL && ++reads>${fault==="after" ? 1 : 0}) {
  FILE *m=fopen(${JSON.stringify(marker)},"w"); if(m){fprintf(m,"%d",getpid());fclose(m);}
  ${fault === "healthy" ? "usleep(20000);" : "for(;;) sleep(1);"}
 }
 return actual(fd,buf,count);
}
`)
 const compiled=spawnSync("cc",["-shared","-fPIC","-o",library,source,"-ldl"],{encoding:"utf8",timeout:10000})
 assert.equal(compiled.status,0,compiled.stderr)
 const moduleUrl=new URL("../apps/kernel/slice-linux-docker/managed-home-archive-digest.mjs",import.meta.url).href
  const script=fault === "healthy"
  ? `import {openSync,closeSync} from 'node:fs';import {digestPinnedHomeArchive} from ${JSON.stringify(moduleUrl)};const fd=openSync(${JSON.stringify(archive)},'r');try{const start=Date.now();const digest=await digestPinnedHomeArchive(fd,200);if(Date.now()-start<=200 || digest!==${JSON.stringify(sha256(contents))})throw Error('healthy progress failed')}finally{closeSync(fd)}`
  : `import {openSync,closeSync} from 'node:fs';import {digestPinnedHomeArchive} from ${JSON.stringify(moduleUrl)};const fd=openSync(${JSON.stringify(archive)},'r');try{await digestPinnedHomeArchive(fd,50);process.exitCode=99}catch(e){if(!/made no progress/.test(e.message))throw e}finally{closeSync(fd)}`
 const result=spawnSync(process.execPath,["--input-type=module","-e",script],{env:{...process.env,LD_PRELOAD:library},encoding:"utf8",timeout:4000})
 assert.equal(result.status,0,result.stderr)
 const pid=(await readFile(marker,"utf8")).trim()
 const state=await readFile(`/proc/${pid}/stat`,"utf8").catch(()=>undefined)
 assert(!state || state.split(") ")[1].startsWith("Z "),"stalled hash worker survived settlement")
})
