// Remove a slice credential and legacy backups whose ownership is proven.
const fs = require('node:fs')
const path = require('node:path')
const C = fs.constants
const MAX_BYTES = 1024 * 1024
function readRegular(file) {
  const fd = fs.openSync(file,C.O_RDONLY|C.O_NOFOLLOW|C.O_NONBLOCK)
  try {
    const stat = fs.fstatSync(fd)
    if(!stat.isFile() || stat.size>MAX_BYTES || stat.nlink!==1)throw new Error('invalid credential file')
    const buffer=Buffer.alloc(MAX_BYTES+1);let size=0
    while(size<buffer.length){const count=fs.readSync(fd,buffer,size,buffer.length-size,null);if(!count)break;size+=count}
    if(size>MAX_BYTES){buffer.fill(0);throw new Error('credential exceeds limit')}
    return buffer.subarray(0,size)
  } finally {fs.closeSync(fd)}
}
function identity(file) {
  const stat=fs.lstatSync(file)
  if(!stat.isFile()||stat.nlink!==1)throw new Error('credential ownership conflict')
  return {dev:stat.dev,ino:stat.ino,ctimeMs:stat.ctimeMs,size:stat.size}
}
function sameIdentity(left,right) {
  return left&&right&&['dev','ino','ctimeMs','size'].every(key=>left[key]===right[key])
}
function optionalIdentity(file) {
  try{return identity(file)}catch(error){if(error.code==='ENOENT')return null;throw error}
}
function readOwnership(parent,name) {
  let bytes
  try{bytes=readRegular(path.join(parent,`${name}.chariox-slice-auth-owned.json`));return JSON.parse(bytes)}
  catch(error){if(error.code==='ENOENT')return null;throw error}finally{bytes?.fill(0)}
}
function removeAuth(target) {
  const directory=path.dirname(target)
  let parent;try {parent=fs.openSync(directory,C.O_RDONLY|C.O_DIRECTORY|C.O_NOFOLLOW)}catch(error){if(error.code==='ENOENT')return;throw error}
  try {
    const pinned=`/proc/self/fd/${parent}`;const name=path.basename(target)
    const backup=path.join(pinned,`${name}.before-slice-auth`)
    const existingBackup=optionalIdentity(backup)
    const ownership=readOwnership(pinned,name)
    if(existingBackup&&!sameIdentity(existingBackup,ownership?.backup))throw new Error('backup ownership conflict')
    for(const file of [path.join(pinned,name),backup,path.join(pinned,`${name}.chariox-slice-auth-owned.json`)]) {
      if(optionalIdentity(file))fs.unlinkSync(file)
    }
    fs.fsyncSync(parent)
  }finally {fs.closeSync(parent)}
}
try {
  const [operation,target]=process.argv.slice(1)
  if(!path.isAbsolute(target))throw new Error('invalid destination')
  if(operation==='remove')removeAuth(target)
  else throw new Error('invalid operation')
}catch {process.stderr.write('slice credential file operation failed\n');process.exitCode=1}
