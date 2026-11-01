// Same probe as guest/src/lib.rs, for componentize-js: an async export that
// awaits an async import.
import { get } from 'spike:probe/kernel@0.1.0';

export async function run(n) {
  let last = '';
  for (let i = 0; i < n; i++) last = await get(`key-${i}`);
  return last;
}
