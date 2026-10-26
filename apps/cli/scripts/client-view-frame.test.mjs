// MP-08 / MP-10 / MP-11: actual native-buffer roundtrip; supplementary, not live acceptance.
import assert from 'node:assert/strict'
import {OptimizedBuffer,RGBA} from '@opentui/core'
import {captureTranscriptFrame,paintCachedTranscriptFrame} from '../dist/client-view-frame.js'
import {validCachedTranscriptFrame} from '../dist/client-view-frame-types.js'
const source=OptimizedBuffer.create(40,8,'wcwidth'),target=OptimizedBuffer.create(40,8,'wcwidth')
try {
 const bg=RGBA.fromInts(17,17,17),fg=RGBA.fromInts(220,180,20)
 source.clear(bg);target.clear(bg)
 source.drawText('approval outside transcript',0,0,fg)
 source.drawText('Answer 漢字 <chariox-context>',1,2,fg,bg,1)
 source.drawText('const x = "$literal";',1,3,RGBA.fromInts(40,190,130),bg,4)
 const frame=captureTranscriptFrame(source,{x:1,y:2,width:35,height:3})
 assert.ok(frame);assert.ok(validCachedTranscriptFrame(frame))
 const text=frame.lines.map(line=>line.spans.map(span=>span.text).join('')).join('\n')
 assert.ok(text.includes('漢字 <chariox-context>'));assert.ok(text.includes('$literal'));assert.ok(!text.includes('approval'))
 paintCachedTranscriptFrame(target,JSON.parse(JSON.stringify(frame)),{x:1,y:2,width:35,height:3})
 assert.deepEqual(captureTranscriptFrame(target,{x:1,y:2,width:35,height:3}),frame)
 assert.equal(validCachedTranscriptFrame({...frame,width:1001}),false)
 const unsafe=structuredClone(frame);unsafe.lines[0].spans[0].text='\u001b[31m';assert.equal(validCachedTranscriptFrame(unsafe),false)
 console.log(JSON.stringify({scope:'MP-08 / MP-10 / MP-11',status:'PASS',checks:6,unicode:true,colors:true,croppedControls:true}))
} finally {source.destroy();target.destroy()}
