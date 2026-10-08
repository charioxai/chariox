"""MD-DISPLAY-02/04: package public runtime/source only, never runtime state.
Usage: python3 build-lan-kit.py <kernel-test-ELF> <node-tools> <pyav-tools> <new-output>
Run only from a clean, committed checkout; no network/build/host mutation.
"""
import hashlib,json,re,shutil,subprocess,sys,sysconfig,tarfile,os
from pathlib import Path
sys.dont_write_bytecode=True
from lan_node_runtime import install as install_node
from protocol_versions import PROTOCOL_FILES,versions
binary,tools,pytools,out=map(Path,sys.argv[1:])
checkout=Path(__file__).resolve().parents[2]
if subprocess.check_output(['git','status','--porcelain'],cwd=checkout).strip():raise SystemExit('MD-DISPLAY: commit source before packaging')
source=subprocess.check_output(['git','rev-parse','HEAD'],cwd=checkout,text=True).strip()
build_source=os.environ.get('MD_KERNEL_BUILD_SOURCE',source)
if not re.fullmatch('[a-f0-9]{40}',build_source):raise SystemExit('MD-DISPLAY: exact kernel build source required')
subprocess.run(['git','cat-file','-e',build_source+'^{commit}'],cwd=checkout,check=True)
# MP-08/MP-10/MP-11: unchanged embedded assets alone cannot attest newer
# Rust/C behavior. Harness-only commits may reuse byte-identical runtime source.
runtime_paths=['Cargo.toml','Cargo.lock','.cargo','apps/kernel','apps/relay','packages']
if subprocess.run(['git','diff','--quiet',build_source,'HEAD','--',*runtime_paths],cwd=checkout).returncode:
 raise SystemExit('MP-08/MP-10/MP-11: kernel runtime sources differ from exact build source')
build_versions=versions(lambda relative:subprocess.check_output(['git','show',build_source+':'+relative],cwd=checkout))
out.mkdir(parents=True,exist_ok=True);kit=out/'display-lan-kit';kit.mkdir(exist_ok=False)
def copy(source,dest):
 dest.parent.mkdir(parents=True,exist_ok=True)
 if source.is_dir():shutil.copytree(source,dest,ignore=shutil.ignore_patterns('__pycache__','test','tests','idlelib','tkinter','ensurepip'))
 else:shutil.copy2(source,dest)
# Fixed public-source allowlist; no daemon/controller configuration or profiles.
files=subprocess.check_output(['git','ls-files','apps/browser-display'],cwd=checkout,text=True).splitlines()
assets=(checkout/'apps/kernel/src/runtime/kernel_browser_assets.rs').read_text()
files += ['apps/kernel/src/runtime/kernel_browser_assets.rs','packages/kernel-client/src/browser-relay-crypto.ts','docs/MULTIDOMAIN_KERNEL_BROWSER.md']
files += [path for path,_ in PROTOCOL_FILES.values()]
files += ['apps/kernel/slice-linux-docker/docker/'+n for n in re.findall(r'include_bytes!\("../../slice-linux-docker/docker/([^"/]+)"\)',assets)]
for name in sorted(set(files)):
 if name.startswith('apps/kernel/'):
  built=subprocess.check_output(['git','show',build_source+':'+name],cwd=checkout)
  if built!=(checkout/name).read_bytes():raise SystemExit('MP-08/MP-10: kernel asset differs from exact build source '+name)
 copy(checkout/name,kit/name)
native_worker=os.environ.get('MD_NATIVE_WORKER')
if native_worker:copy(Path(native_worker),kit/'runtime/native-worker')
copy(binary,kit/'runtime/kernel-tests');subprocess.run(['strip','--strip-debug',str(kit/'runtime/kernel-tests')],check=True)
node=install_node(os.environ['MD_NODE_ARCHIVE'],kit/'runtime/node');copy(Path(sys.executable).resolve(),kit/'runtime/python/bin/python3')
stdlib=Path(sysconfig.get_path('stdlib'));copy(stdlib,kit/'runtime/python/lib'/stdlib.name)
copy(tools,kit/'tools')
for name in ['av','av.libs']:copy(pytools/name,kit/'pytools'/name)
# Bundle loader/core libs so this newer builder's Node/Python/kernel can run on
# Ubuntu's older glibc. Invoke that loader explicitly, never export LD_* globally.
libs=kit/'runtime/lib';libs.mkdir()
roots=[kit/'runtime/node',kit/'runtime/kernel-tests',kit/'runtime/python/bin/python3']+list((kit/'runtime/python').rglob('*.so'))+list((kit/'pytools').rglob('*.so'))
if native_worker:roots.append(kit/'runtime/native-worker')
roots += [Path('/usr/lib/x86_64-linux-gnu')/n for n in ['libX11.so.6','libXext.so.6','libXdamage.so.1','libXcomposite.so.1','libxxhash.so.0']]
for root in roots:
 text=subprocess.run(['ldd',str(root)],capture_output=True,text=True,check=False).stdout
 for name in re.findall(r'(?:=>\s+|^\s*)(/[^\s]+)',text,re.M):
  p=Path(name)
  if p.exists() and not (libs/p.name).exists():copy(p.resolve(),libs/p.name)
# MP-11: Python dlopens xxhash as well as the X libraries; ldd cannot
# discover those imports. Bundle each root and its resolved dependencies.
for root in roots[-5:]:copy(root.resolve(),libs/root.name)
subprocess.run([sys.executable,str(checkout/'apps/browser-display/check-lan-kit.py'),str(kit)],check=True)
loader=libs/'ld-linux-x86-64.so.2'
if not loader.exists():raise SystemExit('MD-DISPLAY: loader missing')
wrappers=kit/'runtime/bin';wrappers.mkdir()
for name,program in [('node','node'),('python3','python/bin/python3')]+([('native-worker','native-worker')] if native_worker else []):
 extra='export PYTHONHOME="$base/python"\n' if name=='python3' else ''
 wrapper=f'#!/bin/sh\nset -eu\nbase=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)\n{extra}exec "$base/lib/ld-linux-x86-64.so.2" --library-path "$base/lib" "$base/{program}" "$@"\n'
 (wrappers/name).write_text(wrapper);(wrappers/name).chmod(0o755)
(kit/'run-lan.sh').write_text('#!/bin/sh\nset -eu\nbase=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$base/runtime/bin/node" "$base/apps/browser-display/lan-run.mjs" "$@"\n');(kit/'run-lan.sh').chmod(0o755)
digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
manifest={'item':'MP-08/MP-10/MP-11 MD-DISPLAY-02/04','source':source,'binary':{'build_source':build_source,'original_sha256':digest(binary),'sha256':digest(kit/'runtime/kernel-tests'),'transform':'strip --strip-debug',**build_versions},'node':node,'python':sys.version.split()[0],'chromium':'host executable, mandatory version recorded by each case','files':[]}
for p in sorted(kit.rglob('*')):
 if p.is_file():manifest['files'].append({'path':str(p.relative_to(kit)),'sha256':digest(p)})
(kit/'KIT_MANIFEST.json').write_text(json.dumps(manifest,indent=2)+'\n')
tar=out/'display-lan-kit.tar.gz'
with tarfile.open(tar,'w:gz',compresslevel=3) as archive:archive.add(kit,arcname=kit.name)
(out/'display-lan-kit.tar.gz.sha256').write_text(digest(tar)+'  '+tar.name+'\n')
print(json.dumps({'item':manifest['item'],'source':source,'tar':str(tar),'sha256':digest(tar),'bytes':tar.stat().st_size}))
