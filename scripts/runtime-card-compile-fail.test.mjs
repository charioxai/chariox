import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

test('code outside the registry cannot construct a live Room card', () => {
  const work = mkdtempSync(join(tmpdir(), 'runtime-card-'))
  try {
    const production = fileURLToPath(new URL('../apps/kernel/src/runtime/state/runtime_interaction_owned_state/card_registry/live_room_interaction.rs', import.meta.url))
    const input = join(work, 'probe.rs')
    const compile = (entry) => {
      writeFileSync(input, `
        pub mod session { pub struct RuntimeInteraction; }
        pub mod registry {
          #[path = ${JSON.stringify(production)}]
          pub mod live_room_interaction;
          pub fn admit() -> live_room_interaction::LiveRoomInteraction {
            live_room_interaction::LiveRoomInteraction::admitted(crate::session::RuntimeInteraction)
          }
        }
        fn main() { ${entry} }
      `)
      return spawnSync('rustc', ['--emit=metadata', '--out-dir', work, input], { encoding: 'utf8' })
    }
    const normal = compile('registry::admit();')
    assert.equal(normal.status, 0, normal.error?.message ?? normal.stderr)
    const raw = compile('let _ = registry::live_room_interaction::LiveRoomInteraction { presentation: session::RuntimeInteraction };')
    assert.notEqual(raw.status, 0)
    assert.match(raw.stderr, /error\[E0451\].*field.*presentation.*private/)
    const bypass = compile('registry::live_room_interaction::LiveRoomInteraction::admitted(session::RuntimeInteraction);')
    assert.notEqual(bypass.status, 0)
    assert.match(bypass.stderr, /error\[E0624\].*associated function.*admitted.*private/)
  } finally {
    rmSync(work, { recursive: true, force: true })
  }
})
