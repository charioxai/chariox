// Usage: node scripts/drills/score-waiting-job-soak.mjs <events.jsonl>
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

export function scoreWaitingJob(samples) {
  const first = samples[0]?.at
  const last = samples.at(-1)?.at
  assert(last - first >= 3590_000, 'need one hour of samples at ten-second intervals')
  assert(samples.slice(1).every((sample, index) => sample.at - samples[index].at <= 20_000),
    'memory samples must cover the hour without gaps')
  const warm = samples.filter(sample => sample.at - first >= 60_000)
  const rss = warm.map(sample => {
    assert(Number.isFinite(sample.kernelRss), 'missing kernel RSS')
    const job = sample.tree.find(group => group.members.some(process => process.comm === 'sleep'))
    assert(job && job.members.every(process => Number.isFinite(process.rss_kib)), 'missing waiting job RSS')
    return { at: sample.at, kernel: sample.kernelRss, job: job.members.reduce((sum, process) => sum + process.rss_kib, 0) }
  })
  for (const [key, limit] of [['kernel', 32 * 1024], ['job', 2 * 1024]]) {
    let elevatedSince
    for (const sample of rss) {
      if (sample[key] - rss[0][key] <= limit) {
        elevatedSince = undefined
        continue
      }
      elevatedSince ??= sample.at
      assert(sample.at - elevatedSince < 120_000, `sustained ${key} RSS growth exceeded ${limit} KiB`)
    }
  }
  return { samples: samples.length, start: rss[0], end: rss.at(-1) }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const samples = readFileSync(process.argv[2], 'utf8').trim().split('\n')
    .map(line => JSON.parse(line)).filter(event => event.kind === 'soak')
  console.log(JSON.stringify(scoreWaitingJob(samples)))
}
