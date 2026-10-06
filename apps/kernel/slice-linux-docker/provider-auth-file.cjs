// Private slice-owned credential publication. This program is sent as code; stdin carries auth.
const fs = require('node:fs')
const path = require('node:path')
const {randomBytes} = require('node:crypto')
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
function writePrivate(parent,name,bytes) {
  const temp=`.chariox-auth-${randomBytes(16).toString('hex')}.tmp`
  const target=path.join(parent,temp)
  const fd=fs.openSync(target,C.O_WRONLY|C.O_CREAT|C.O_EXCL|C.O_NOFOLLOW,0o600)
  const created=fs.fstatSync(fd)
  try {fs.fchmodSync(fd,0o600);fs.writeFileSync(fd,bytes);fs.fsyncSync(fd);fs.renameSync(target,path.join(parent,name))}
  finally {
    fs.closeSync(fd)
    const current=optionalIdentity(target)
    if(current&&current.dev===created.dev&&current.ino===created.ino)fs.unlinkSync(target)
  }
}
function importAuth(target) {
  const directory=path.dirname(target);fs.mkdirSync(directory,{recursive:true,mode:0o700})
  const parent=fs.openSync(directory,C.O_RDONLY|C.O_DIRECTORY|C.O_NOFOLLOW)
  const pinned=`/proc/self/fd/${parent}`;const name=path.basename(target)
  let incoming,previous
  try {
    const backup=path.join(pinned,`${name}.before-slice-auth`)
    const ownership=readOwnership(pinned,name)
    const existingBackup=optionalIdentity(backup)
    if(existingBackup&&!sameIdentity(existingBackup,ownership?.backup))throw new Error('backup ownership conflict')
    try {previous=readRegular(path.join(pinned,name))}catch(error){if(error.code!=='ENOENT')throw error}
    incoming=Buffer.alloc(MAX_BYTES+1);let size=0
    while(size<incoming.length){const count=fs.readSync(0,incoming,size,incoming.length-size,null);if(!count)break;size+=count}
    if(!size || size>MAX_BYTES)throw new Error('credential input is invalid')
    if(previous&&!existingBackup)writePrivate(pinned,`${name}.before-slice-auth`,previous)
    // Persist backup ownership before replacing active auth. A crash cannot
    // turn an unknown existing backup into something cleanup may delete.
    const state={backup:optionalIdentity(backup)}
    writePrivate(pinned,`${name}.chariox-slice-auth-owned.json`,Buffer.from(JSON.stringify(state)))
    fs.fsyncSync(parent)
    writePrivate(pinned,name,incoming.subarray(0,size));fs.fsyncSync(parent)
  }finally {incoming?.fill(0);previous?.fill(0);fs.closeSync(parent)}
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
  if(operation==='import')importAuth(target)
  else if(operation==='remove')removeAuth(target)
  else throw new Error('invalid operation')
}catch {process.stderr.write('slice credential file operation failed\n');process.exitCode=1}
