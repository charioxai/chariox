#!/usr/bin/env node
// MP-08 / MP-10 / MP-11 H10: admit allowlisted public health receipts only.
import { readFile, open, realpath } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { admitSites } from './admission.mjs'
const [input,output]=process.argv.slice(2)
if(!path.isAbsolute(input??'') || !path.isAbsolute(output??''))throw new Error('MP-10 absolute public receipt/external output required')
const publicFields=['generation','sourceDigest','resetAtMs','nowMs','maxAgeMs','shops']
const bytes=await readFile(input);if(bytes.length>65536)throw new Error('MP-10 bounded public readiness receipt required')
const data=JSON.parse(bytes)
if(Object.keys(data).some(key=>!publicFields.includes(key)))throw new Error('MP-10 readiness receipt contains unapproved fields')
const result=admitSites({...data,nowMs:Date.now()})
const root=await realpath(path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../../../..'))
const parent=await realpath(path.dirname(output));const relative=path.relative(root,parent)
if(!relative || (!relative.startsWith(`..${path.sep}`)&&relative!=='..'&&!path.isAbsolute(relative)))throw new Error('MP-10 external evidence required')
const file=await open(path.join(parent,path.basename(output)),'wx',0o600)
try{await file.writeFile(JSON.stringify(result,null,2)+'\n');await file.sync()}finally{await file.close()}
console.log('MP-08 / MP-10 / MP-11 public four-shop readiness receipts admitted; no prompt or score produced')
