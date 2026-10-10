// MD-DISPLAY-02/04: real codec/decoder across coalesced A/B/A source work.
import test from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {MotionEncoder} from './kernel-browser-motion.mjs';
import {DisplayStream, PortableEncoder} from './kernel-browser-display.mjs';

for (const codec of ['avc1.420033', 'vp09.00.40.08']) test(`MD-DISPLAY ${codec} coalesced A/B/A retains the real decoder reference chain`, async () => {
  const portable = new PortableEncoder();
  let release, entered;
  const started = new Promise(resolve => entered = resolve);
  const hold = new Promise(resolve => release = resolve);
  let calls = 0, offer;
  const encoder = {async encode(...args) {
    if (++calls === 1) { entered(); await hold; }
    return portable.encode(...args);
  }, close: () => portable.close()};
  const source = {subscribe(fn) { offer = fn; return () => {}; }, sample: () => null};
  const producer = new MotionEncoder(source, encoder, {codec, bitrate: 2_000_000});
  const stream = new DisplayStream({subscription_id:'s', tab_id:'t', device_scale_factor:1,
    bitrate:2_000_000, codec, dependencies:true}, {encoder, now:()=>0, wait:async()=>{}});
  stream.document_id = 'd';
  const image = (serial, value) => ({serial, motion:true, width:128, height:128, generation:1,
    data_base64:String(value), raw:{width:128, height:128, format:'bgr0', length:128*128*4, pixels:Buffer.alloc(128*128*4, value)}});
  try {
    offer(image(1, 0)); await started;
    offer(image(2, 40)); offer(image(3, 0)); release(); await producer.active;
    const packets = [];
    for (let n=0; n<2; n++) {
      const frame = await stream.frame(producer.take(), 'd', stream.sequence);
      assert.ok(frame, 'already encoded packets are dependencies even if source pixels repeat');
      packets.push(frame.data_base64);
    }
    offer(image(4, 200)); await producer.active;
    packets.push((await stream.frame(producer.take(), 'd', stream.sequence)).data_base64);
    const decoded = JSON.parse(execFileSync(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON || 'python3', ['-c',
      `import av,base64,json,sys
d=av.CodecContext.create('${codec.startsWith('avc')?'h264':'vp9'}','r')
out=[]
for p in json.load(sys.stdin):
 for f in d.decode(av.Packet(base64.b64decode(p))):
  out.append(list(bytes(f.reformat(format='rgb24').planes[0])[:3]))
print(json.dumps(out))`], {input:JSON.stringify(packets), encoding:'utf8'}));
    assert.equal(decoded.length, 3);
    assert.ok(decoded[2].every(value => Math.abs(value-200)<12), 'changed frame reconstructs after the duplicate delta');
  } finally { release(); await producer.close(); await stream.close(); }
});
