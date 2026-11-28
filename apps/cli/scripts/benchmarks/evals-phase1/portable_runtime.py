#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: hash-bound public ELF loader packaging for official task images."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
from turn import file_hash, preflight


def package(source, output, patchelf, protocol):
    manifest=json.loads((source/'eval-runtime.json').read_text())
    preflight(str(source),manifest['source_commit'],manifest['kernel_sha256'],protocol)
    if output.exists() or not output.is_absolute():
        raise ValueError('MP-11: new absolute lane-owned runtime directory required')
    executables=[source/'bin/chariox-kernel',source/'bin/chariox-relay']
    executables += [p for p in source.glob('providers/codex/**/zsh') if p.is_file()]
    libraries={}
    for executable in executables:
        dependencies=subprocess.check_output(['ldd',str(executable)],text=True)
        for path in re.findall(r'(?:=>\s*)?(/[^\s]+)',dependencies):
            p=Path(path)
            if p.is_file():
                if p.name in libraries and libraries[p.name]!=p.resolve():
                    raise ValueError('MP-11: conflicting public dependency names')
                libraries[p.name]=p.resolve()
    if 'ld-linux-x86-64.so.2' not in libraries:
        raise ValueError('MP-11: public ELF interpreter missing')
    shutil.copytree(source,output,symlinks=True)
    lib=output/'lib';lib.mkdir()
    for name,path in libraries.items():shutil.copy2(path,lib/name)
    patched=[]
    for original in executables:
        relative=original.relative_to(source);target=output/relative
        subprocess.run([str(patchelf),'--set-interpreter',str(lib/'ld-linux-x86-64.so.2'),
                        '--set-rpath','$ORIGIN/'+os.path.relpath(lib,target.parent),str(target)],check=True)
        patched.append({'path':str(relative),'original_sha256':file_hash(original),'packaged_sha256':file_hash(target)})
    manifest['kernel_sha256']=file_hash(output/'bin/chariox-kernel')
    manifest['packaging']={'mp_items':['MP-08','MP-10','MP-11'],'method':'public native ELF interpreter and library bundle; no source changes',
                           'requires_same_absolute_mount':True,'source_manifest_sha256':file_hash(source/'eval-runtime.json'),
                           'patchelf_sha256':file_hash(patchelf),'patched':patched,
                           'public_libraries':{name:{'source':str(path),'sha256':file_hash(path)} for name,path in libraries.items()}}
    manifest['files']={str(p.relative_to(output)):file_hash(p) for p in output.rglob('*') if p.is_file() and p!=output/'eval-runtime.json'}
    (output/'eval-runtime.json').write_text(json.dumps(manifest,indent=2)+'\n')
    preflight(str(output),manifest['source_commit'],manifest['kernel_sha256'],protocol)
    return manifest


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['source','output','patchelf']:parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--local-protocol',type=int,required=True)
    args=parser.parse_args();result=package(args.source,args.output,args.patchelf,args.local_protocol)
    print('MP-08 / MP-10 / MP-11:',result['source_commit'],result['kernel_sha256'])
