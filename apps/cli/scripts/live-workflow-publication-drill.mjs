#!/usr/bin/env node
import { runLiveWorkflowPublicationDrill } from './lib/live-workflow-publication-drill-flow.mjs'

const args = process.argv.slice(2)
if (args.includes('--help')) {
  console.log('Usage: node apps/cli/scripts/live-workflow-publication-drill.mjs')
  process.exit(0)
}
if (args.length > 0) {
  console.error(`unknown argument: ${args[0]}`)
  process.exit(2)
}

runLiveWorkflowPublicationDrill().catch((error) => {
  console.error(`[publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
