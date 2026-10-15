// MP-08 / MP-10 / MP-11: source-level binding completeness, supplementary to compilers.
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const rust = readFileSync(new URL('../apps/kernel/src/skill/v2.rs', import.meta.url), 'utf8');
const ts = readFileSync(new URL('../packages/kernel-client/src/kernel-types-skills-v2.ts', import.meta.url), 'utf8');
const swift = readFileSync(new URL('../apps/ios/CharioxPackage/Sources/CharioxFeature/Kernel/SkillsV2Protocol.swift', import.meta.url), 'utf8');
let checked = 0;
for (const match of rust.matchAll(/pub (struct|enum) (\w+)\s*\{/g)) {
  const [, kind, name] = match;
  let end = match.index + match[0].length, depth = 1;
  const start = end;
  for (; depth; end++) { if (rust[end] === '{') depth++; if (rust[end] === '}') depth--; }
  const body = rust.slice(start, end - 1);
  assert(ts.includes(`export type ${name} =`), `TS missing ${name}`);
  assert(swift.includes(`public ${kind} ${name}:`), `Swift missing ${name}`);
  if (kind === 'struct') {
    for (const field of body.matchAll(/pub (\w+):/g)) {
      assert(ts.includes(`${field[1]}:`), `TS missing ${name}.${field[1]}`);
      assert(swift.includes(`public var ${field[1]}:`), `Swift missing ${name}.${field[1]}`);
    }
  }
  checked++;
}
assert(ts.includes('SKILLS_MIN_LOCAL_PROTOCOL = 533'));
assert(ts.includes('SKILLS_MIN_PEER_PROTOCOL = 141'));
assert(swift.includes('skillsMinimumLocalProtocol: UInt32 = 533'));
assert(swift.includes('skillsMinimumPeerProtocol: UInt32 = 141'));
console.log(`MP-08 / MP-10 / MP-11: ${checked} Skills v2 Rust/TS/Swift wire types covered (source guard; Swift compilation separate)`);
