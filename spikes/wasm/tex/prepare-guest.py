#!/usr/bin/env python3
"""MP-08/MP-11: reuse the pinned fork driver without vendoring upstream code."""
from pathlib import Path
import json, sys, tomllib
here=Path(__file__).resolve().parent
root=Path(sys.argv[1]).resolve(); fork=root/'tectonic'; out=root/'guest'; (out/'src').mkdir(parents=True,exist_ok=True)
s=(here/'guest.rs').read_text().replace('/root/work/agent-texspike/spikes/wasm/wit',str(here.parent/'wit'))
(out/'src/lib.rs').write_text(s)
s=(fork/'crates/wasi/src/driver.rs').read_text().replace('"/cache"','"/data/cache"').replace('"/fonts"','"/data/bundle"')
(out/'src/driver.rs').write_text(s)
up=tomllib.loads((fork/'crates/wasi/Cargo.toml').read_text())
lines=['[package]','name = "texspike-guest"','version = "0.1.0"','edition = "2021"','[lib]','crate-type = ["cdylib"]','[workspace]','[dependencies]','wit-bindgen = "=0.63.0"','serde_json = "1.0.140"']
for name,spec in up['dependencies'].items():
 if isinstance(spec,str): lines.append(name+' = '+json.dumps(spec))
 else:
  spec=dict(spec)
  if 'path' in spec: spec['path']=str((fork/'crates/wasi'/spec['path']).resolve())
  lines.append(name+' = { '+', '.join(k+' = '+json.dumps(v) for k,v in spec.items())+' }')
lines += ['[profile.release]','opt-level = "s"','lto = true','strip = true','codegen-units = 1']
(out/'Cargo.toml').write_text('\n'.join(lines)+'\n')

(out/'Cargo.lock').write_bytes((here/'guest.Cargo.lock').read_bytes())
