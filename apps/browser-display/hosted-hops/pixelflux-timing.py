# MP-08/MP-10/MP-11: numeric X11 capture/queue/encode timestamps for pinned own baseline.
from pathlib import Path
import sys,hashlib,json
root=Path(sys.argv[1]);output=Path(sys.argv[2]);assert str(output).startswith('/root/.chariox/dev/vidlat/r6/')
p=root/'pixelflux/src/x11/mod.rs';s=p.read_text();original=hashlib.sha256(s.encode()).hexdigest()
def change(old,new):
 global s
 assert s.count(old)==1,'MP-10 pinned pixelflux seam changed: '+old[:45]
 s=s.replace(old,new)
change('struct RawFrame {\n', 'struct RawFrame {\n    vidlat_capture_start: f64,\n    vidlat_capture_end: f64,\n    vidlat_published: f64,\n')
change('            let grab_result = {','            let vidlat_capture_start = vidlat_now();\n            let grab_result = {')
change('            if let Err(e) = grab_result {','            let vidlat_capture_end = vidlat_now();\n            if let Err(e) = grab_result {')
change('                RawFrame {\n                    idx,','                RawFrame {\n                    vidlat_capture_start,\n                    vidlat_capture_end,\n                    vidlat_published: vidlat_now(),\n                    idx,')
change('        RawFrame {\n            idx,','        RawFrame {\n            vidlat_capture_start: 0.0,\n            vidlat_capture_end: 0.0,\n            vidlat_published: 0.0,\n            idx,')
change('        let stripes = pl.process(buf, frame.stride);','''        let vidlat_encode_start = vidlat_now();
        let stripes = pl.process(buf, frame.stride);
        vidlat_trace(&frame, vidlat_encode_start, vidlat_now(), stripes.first().map(|s| s.frame_id));''')
s+='''
// MP-08/MP-10/MP-11 local observer. No pixels/input text or protocol changes.
fn vidlat_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()*1000.0
}
fn vidlat_trace(frame: &RawFrame, encode_start: f64, encode_end: f64, id: Option<i32>) {
    use std::io::Write;
    if let Ok(path) = std::env::var("VIDLAT_PIXELFLUX_TIMING") {
        if !path.starts_with("/root/.chariox/dev/vidlat/r6/") { return; }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{{\\"stage\\":\\"native_capture_encode\\",\\"capture_start_ms\\":{},\\"capture_end_ms\\":{},\\"published_ms\\":{},\\"encode_start_ms\\":{},\\"encode_end_ms\\":{},\\"frame_id\\":{},\\"empty\\":{}}}", frame.vidlat_capture_start, frame.vidlat_capture_end, frame.vidlat_published, encode_start, encode_end, id.unwrap_or(-1), if id.is_none() {1}else{0});
        }
    }
}
'''
output.parent.mkdir(parents=True,exist_ok=True);output.write_text(s)
output.with_suffix('.receipt.json').write_text(json.dumps({'items':['MP-08','MP-10','MP-11'],'original_sha256':original,'instrumented_sha256':hashlib.sha256(s.encode()).hexdigest(),'qualification':'pinned own measurement-only Rust X11 capture/queue/encode observer; not built or measured yet. File-I/O overhead requires paired uninstrumented control.'},indent=2))
print('MP-08/MP-10/MP-11 pinned native baseline trace patch generated')
