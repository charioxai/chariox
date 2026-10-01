import assert from "node:assert/strict"
import { mkdtemp, mkdir, writeFile, readFile, chmod, rm } from "node:fs/promises"
import { spawnSync } from "node:child_process"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { fileURLToPath } from "node:url"
import { test } from "node:test"

const provisioner = fileURLToPath(new URL("../apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", import.meta.url))
const digest = "sha256:" + "a".repeat(64)
const relaySource = await readFile(fileURLToPath(new URL("../apps/kernel/src/transport/relay_peer.rs", import.meta.url)), "utf8")
const protocol = relaySource.match(/pub const RELAY_PEER_PROTOCOL_VERSION: u32 = ([0-9]+);/)[1]

async function fixture(context, images = []) {
  const root = await mkdtemp(join(tmpdir(), "chariox-extension-policy-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const bin = join(root, "bin")
  const source = join(root, "context")
  await mkdir(bin)
  await mkdir(source)
  await writeFile(join(source, "Customfile"), "ARG CHARIOX_SLICE_BASE_IMAGE\nFROM $CHARIOX_SLICE_BASE_IMAGE\n")
  const calls = join(root, "calls.jsonl")
  const docker = join(bin, "docker")
  await writeFile(docker, `#!${process.execPath}
import {appendFileSync} from "node:fs"
if(process.env.FIXTURE_ENGINE_CONTRACT==="1") {
 if(process.env.DOCKER_HOST!=="unix:///synthetic-slice.sock" || ["DOCKER_CONTEXT","BUILDX_BUILDER","BUILDX_HOST","BUILDKIT_HOST","BASH_ENV"].some(k=>process.env[k])) process.exit(98)
 if(process.env.ARBITRARY_HELPER_SETTING!=="kept") process.exit(97)
}
const args=process.argv.slice(2)
appendFileSync(process.env.FIXTURE_CALLS,JSON.stringify(args)+"\\n")
if(args[0]==="info") console.log(args.includes("--format")?"amd64":"ready")
else if(args[0]==="image"&&args[1]==="inspect") {
 const exists=JSON.parse(process.env.FIXTURE_IMAGES).includes(args.at(-1))
 if(!exists) process.exit(1)
 const format=args[args.findIndex(a=>a==="--format"||a==="-f")+1]
 if(format?.includes("relay-peer-protocol")) console.log(process.env.FIXTURE_PROTOCOL)
 else if(format?.includes("runtime-source-revision")) console.log(process.env.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST)
 else console.log("{}")
} else if(args[0]==="buildx"&&["version","build"].includes(args[1])) {}
else {console.error("unexpected Docker fixture call");process.exit(99)}
`)
  await chmod(docker, 0o755)
  return {
    root, source, calls,
    env: { ...process.env, PATH: bin + ":" + process.env.PATH,
      CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: digest,
      CHARIOX_SLICE_DOCKER_IMAGE: "chariox-slice-extension:creation-a",
      CHARIOX_SLICE_EXTENSION_DOCKERFILE: join(source, "Customfile"),
      FIXTURE_PROTOCOL: protocol, FIXTURE_CALLS: calls, FIXTURE_IMAGES: JSON.stringify(images),
    },
    run(extra={}) { return spawnSync("/bin/bash",["-p",provisioner,"build-image"],{env:{...this.env,...extra},encoding:"utf8",timeout:10000}) },
    async commands() { return (await readFile(calls,"utf8")).trim().split("\n").map(JSON.parse) },
  }
}

test("extension image build retains native context and selected Dockerfile basename", async(context)=>{
 const f=await fixture(context,["chariox-slice-linux:0.1.0"])
 const result=f.run({CHARIOX_SLICE_BUILD_IMAGE:"auto"})
 assert.equal(result.status,0,result.stderr)
 const builds=(await f.commands()).filter(a=>a[0]==="buildx"&&a[1]==="build")
 assert.equal(builds.length,1)
 assert.equal(builds[0].at(-1),f.source)
 assert.equal(builds[0][builds[0].indexOf("-f")+1],join(f.source,"Customfile"))
 assert.equal(builds[0][builds[0].indexOf("-t")+1],"chariox-slice-extension:creation-a")
 assert(builds[0].includes("CHARIOX_SLICE_BASE_IMAGE=chariox-slice-linux:0.1.0"))
})

test("compatible cache skips absent Dockerfile for auto and never",async(context)=>{
 for(const policy of ["auto","never"]) {
  const f=await fixture(context,["chariox-slice-extension:creation-a"])
  const result=f.run({CHARIOX_SLICE_BUILD_IMAGE:policy,CHARIOX_SLICE_EXTENSION_DOCKERFILE:join(f.root,"missing","Customfile")})
  assert.equal(result.status,0,result.stderr)
  assert.equal((await f.commands()).some(a=>a[0]==="buildx"),false)
 }
})

test("preflight separates cache selection from opening a caller Dockerfile",async(context)=>{
 const f=await fixture(context,["chariox-slice-linux:0.1.0"])
 const result=f.run({CHARIOX_SLICE_BUILD_IMAGE:"auto",CHARIOX_SLICE_IMAGE_BUILD_CHECK_ONLY:"1",CHARIOX_SLICE_EXTENSION_DOCKERFILE:join(f.root,"absent")})
 assert.equal(result.status,42,result.stderr)
 assert.equal((await f.commands()).some(a=>a[0]==="buildx"),false)
})

test("never refuses a missing derived image without rebuilding the base",async(context)=>{
 const f=await fixture(context,["chariox-slice-linux:0.1.0"])
 const result=f.run({CHARIOX_SLICE_BUILD_IMAGE:"never"})
 assert.notEqual(result.status,0)
 assert.match(result.stderr,/does not exist and build policy is never/)
 assert.equal((await f.commands()).some(a=>a[0]==="buildx"),false)
})

test("private Dockerfile view can use the original independent build context",async(context)=>{
 const f=await fixture(context,["chariox-slice-linux:0.1.0"])
 const view=join(f.root,"dockerfile-view");await mkdir(view)
 await writeFile(join(view,"Customfile"),"FROM scratch\n")
 const result=f.run({CHARIOX_SLICE_BUILD_IMAGE:"auto",CHARIOX_SLICE_EXTENSION_DOCKERFILE:join(view,"Customfile"),CHARIOX_SLICE_EXTENSION_BUILD_CONTEXT:f.source})
 assert.equal(result.status,0,result.stderr)
 const build=(await f.commands()).find(a=>a[0]==="buildx"&&a[1]==="build")
 assert.equal(build.at(-1),f.source)
 assert.equal(build[build.indexOf("-f")+1],join(view,"Customfile"))
})

const broker = fileURLToPath(new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs", import.meta.url))
function validate(request, root) {
 return spawnSync(process.execPath,[broker,"--validate-request"],{input:JSON.stringify(request),encoding:"utf8",env:{...process.env,CHARIOX_SLICE_DOCKER_SHARE_ROOT:root}})
}
test("typed image positions accept ordinary tags, registries, ports and digests",async(context)=>{
 const root=await mkdtemp(join(tmpdir(),"chariox-extension-image-ref-"));context.after(()=>rm(root,{recursive:true,force:true}))
 for(const image of ["my-slice:Tools_1","registry.example:5000/team/tool:tag","[::1]:5000/team/tool:tag","team/tool@sha256:"+"a".repeat(64)]) {
  assert.equal(validate({kind:"docker",args:["image","inspect","--format","{{.Id}}",image]},root).status,0,image)
  assert.equal(validate({kind:"provisioner",action:"recover",environment:{CHARIOX_SLICE_ID:"slice-dev",CHARIOX_SLICE_NAME:"chariox-slice-dev",CHARIOX_SLICE_HOME_VOLUME:"chariox-slice-dev-home",CHARIOX_SLICE_DOCKER_IMAGE:image},files:[]},root).status,0,image)
 }
})
test("image references cannot widen Docker mutation or raw path positions",async(context)=>{
 const root=await mkdtemp(join(tmpdir(),"chariox-extension-image-ref-denied-"));context.after(()=>rm(root,{recursive:true,force:true}))
 for(const image of ["--privileged","/home/user/context","../image","https://registry/image","image\nflag","registry/UPPER","repo:","repo@sha256:abcd"]) {
  assert.notEqual(validate({kind:"docker",args:["image","inspect","--format","{{.Id}}",image]},root).status,0,image)
 }
 for(const args of [["image","rm","-f","custom:image"],["commit","chariox-slice-dev","custom:image"],["start","custom:image"],["volume","rm","custom:image"],["image","inspect","--format","{{json .}}","custom:image"]]) {
  assert.notEqual(validate({kind:"docker",args},root).status,0,JSON.stringify(args))
 }
})

test("broker consumes a prepared explicit extension image with signed policy and pinned workspace",async(context)=>{
 if(process.platform!=="linux"||process.env.CHARIOX_RUN_PRIVILEGED_MOUNT_TESTS!=="1"){context.skip("explicit Linux mount namespace fixture");return}
 const root=await mkdtemp(join(tmpdir(),"chariox-extension-broker-"))
 const share=join(root,"share"),workspace=join(share,"slices/development/slice-dev/development/workspace")
 const quota=join(root,"quota"),run=join(root,"run"),calls=join(root,"docker-calls")
 await mkdir(workspace,{recursive:true});await mkdir(quota,{mode:0o700});await mkdir(run,{mode:0o700})
 await writeFile(join(workspace,"value"),"pinned-workspace")
 await writeFile(join(quota,"reservations.json"),JSON.stringify({schemaVersion:1,nextProjectId:1073741824,reservations:{}}),{mode:0o600})
 const image="registry.example:5000/team/slice:Tools_1"
 const fake=join(root,"docker")
 await writeFile(fake,`#!/bin/sh
set -eu
printf '%s\\n' "$*" >> '${calls}'
case "$*" in
 'info') exit 0 ;;
 'container inspect --format {{json .Config.Labels}} chariox-slice-dev'|'container inspect --format {{json .Mounts}} chariox-slice-dev')
 printf 'Error: No such container: chariox-slice-dev\\n' >&2;exit 1 ;;
 'volume inspect --format {{json .Labels}} chariox-slice-dev-home')
 printf 'Error: No such volume: chariox-slice-dev-home\\n' >&2;exit 1 ;;
 'image inspect ${image}') printf '{}\\n' ;;
 *relay-peer-protocol-version*) printf '%s\\n' '${protocol}' ;;
 *runtime-source-revision*) printf '%s\\n' '${digest}' ;;
 *) printf 'unexpected Docker operation: %s\\n' "$*" >&2;exit 99 ;;
esac
`)
 await chmod(fake,0o755)
 const wrapper=join(root,"provisioner")
 await writeFile(wrapper,`#!/bin/sh
set -eu
test "$1" = provision
test "$CHARIOX_SLICE_DOCKER_IMAGE" = '${image}'
test "$CHARIOX_SLICE_BUILD_IMAGE" = never
test -z "\${CHARIOX_SLICE_EXTENSION_DOCKERFILE:-}"
test "$CHARIOX_SLICE_BUILD_CONTEXT_DIGEST" = '${digest}'
mountpoint -q -- "$CHARIOX_SLICE_WORKSPACE_SOURCE"
/bin/bash '${provisioner}' build-image
cat "$CHARIOX_SLICE_WORKSPACE_SOURCE/value"
`)
 await chmod(wrapper,0o755)
 const entry=join(root,"namespace")
 await writeFile(entry,`#!/bin/sh
set -eu
mount --bind "$1" /usr/bin/docker
mount --bind "$2" /var/lib/chariox-slice-disk-quota
mount --bind "$3" /run
exec "$4" "$5" --stdio
`)
 await chmod(entry,0o755)
 const manifest=join(root,"release-manifest.json")
 await writeFile(manifest,JSON.stringify({artifacts:[{name:"chariox-slice-build-context",path:"/usr/lib/chariox/slice-build-context",sha256:digest}]}))
 const result=spawnSync("/usr/bin/unshare",["--mount","--propagation","private","--pid","--fork","--mount-proc","--kill-child=KILL",entry,fake,quota,run,process.execPath,broker],{
  input:JSON.stringify({kind:"provisioner",action:"provision",environment:{
   CHARIOX_SLICE_NAME:"chariox-slice-dev",CHARIOX_SLICE_ID:"slice-dev",CHARIOX_SLICE_HOME_VOLUME:"chariox-slice-dev-home",
   CHARIOX_SLICE_OWNER_KERNEL_ID:"kernel-dev",CHARIOX_SLICE_OWNER_MACHINE_ID:"machine-dev",
   CHARIOX_SLICE_WORKSPACE:workspace,CHARIOX_SLICE_DOCKER_IMAGE:image,CHARIOX_SLICE_BUILD_IMAGE:"never"
  },files:[]})+"\n",encoding:"utf8",timeout:15000,env:{...process.env,
   CHARIOX_SLICE_DOCKER_SHARE_ROOT:share,CHARIOX_SLICE_DOCKER_PROVISIONER:wrapper,
   CHARIOX_SLICE_DOCKER_HANDLE_ROOT:join(root,"handles"),CHARIOX_SLICE_DOCKER_HANDLE_STATE:join(root,"handles.json"),
   CHARIOX_MANAGED_RELEASE_MANIFEST:manifest
  }})
 try {
  assert.equal(result.status,0,result.stderr)
  const response=JSON.parse(result.stdout)
  assert.equal(response.status,0,Buffer.from(response.stderrBase64,"base64").toString())
  assert.equal(Buffer.from(response.stdoutBase64,"base64").toString(),"pinned-workspace")
  assert((await readFile(calls,"utf8")).split("\n").some(line=>line.startsWith("image inspect ")&&line.endsWith(image)))
  assert.doesNotMatch(await readFile(calls,"utf8"),/buildx|build |image rm|commit/)
 } finally {
  assert.notEqual(spawnSync("/usr/bin/mountpoint",["-q","--",join(root,"handles")]).status,0)
  await rm(root,{recursive:true,force:true})
 }
})
test("signed extension helper request, descriptor, supervision and projection fixtures",()=>{
 const helper=fileURLToPath(new URL("../apps/kernel/slice-linux-docker/managed-extension-build.test.py",import.meta.url))
 const result=spawnSync("python3",["-I","-S","-B",helper],{encoding:"utf8",env:process.env,timeout:20000})
 assert.equal(result.status,0,result.stderr)
 assert.match(result.stderr,/Ran 19 tests/)
})
test("privileged Python startup ignores caller import and site hooks",async(context)=>{
 const root=await mkdtemp(join(tmpdir(),"chariox-extension-startup-"));context.after(()=>rm(root,{recursive:true,force:true}))
 const marker=join(root,"hook-ran")
 await writeFile(join(root,"sitecustomize.py"),`open(${JSON.stringify(marker)},"w").write("ran")\n`)
 const source=fileURLToPath(new URL("../apps/kernel/slice-linux-docker/managed-extension-build.py",import.meta.url))
 const result=spawnSync("/usr/bin/python3",["-I","-S","-B","-c","import runpy,sys;runpy.run_path(sys.argv[1],run_name='startup_fixture')",source],{encoding:"utf8",env:{...process.env,PYTHONPATH:root,PYTHONHOME:root},timeout:3000})
 assert.equal(result.status,0,result.stderr)
 assert.equal(await readFile(marker).then(()=>true,()=>false),false)
})

test("snapshot-helper consumes a typed custom image without widening helper or volume ownership",async(context)=>{
 const root=await mkdtemp(join(tmpdir(),"chariox-extension-snapshot-image-"));context.after(()=>rm(root,{recursive:true,force:true}))
 const helper="chariox-slice-dev-disk-admission-0123456789abcdef"
 const args=["create","--name",helper,"--memory","512m","--cpus","1","--pids-limit","64","--network","none",
  "--label","io.chariox.snapshot-helper="+helper,"--user","root","-v","chariox-slice-dev-home:/home-src:ro",
  "registry.example:5000/team/slice:Tools_1","sleep","infinity"]
 assert.equal(validate({kind:"docker",args},root).status,0)
 for(const [index,value] of [[2,"foreign-helper"],[16,"foreign-home:/home-src:ro"],[16,"chariox-slice-other-home:/home-src:ro"],[17,"--privileged"]]){
  const denied=[...args];denied[index]=value;assert.notEqual(validate({kind:"docker",args:denied},root).status,0)
 }
})

test("MP-08 MP-11 builds use the configured slice engine despite caller builder overrides", async context => {
 const f=await fixture(context,["chariox-slice-linux:0.1.0"])
 const hook=join(f.root,"startup"), marker=join(f.root,"startup-ran")
 await writeFile(hook,`touch '${marker}'\n`)
 const result=f.run({FIXTURE_ENGINE_CONTRACT:"1",ARBITRARY_HELPER_SETTING:"kept",BASH_ENV:hook,DOCKER_CONTEXT:"foreign",CHARIOX_SLICE_BUILD_IMAGE:"always",BUILDX_BUILDER:"foreign",BUILDX_HOST:"tcp://foreign",BUILDKIT_HOST:"tcp://foreign",DOCKER_HOST:"unix:///synthetic-slice.sock"})
 assert.equal(result.status,0,result.stderr)
 const builds=(await f.commands()).filter(a=>a[0]==="buildx"&&a[1]==="build")
 assert(builds.length>0)
 for(const build of builds) assert.equal(build[build.indexOf("--builder")+1],"default")
 assert.match(result.stderr,/slice engine.*BUILDX_BUILDER/)
 assert.match(result.stderr,/slice engine.*BUILDKIT_HOST/)
 await assert.rejects(readFile(marker),{code:"ENOENT"})
})

test("MP-08 MP-11 raw Docker controls share explicit engine selection with builds", async () => {
 const source=await readFile(new URL("../apps/kernel/src/slice/local_docker/broker.rs",import.meta.url),"utf8")
 const command=source.slice(source.indexOf("fn local_command(&self)"),source.indexOf("#[cfg(all(test, unix))]",source.indexOf("fn local_command(&self)")))
 assert.match(command,/env_remove\("DOCKER_CONTEXT"\)/)
})
