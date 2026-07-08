#!/usr/bin/env node
import { runLiveWorkflowPublicationDrill } from './lib/live-workflow-publication-drill-flow.mjs'

runLiveWorkflowPublicationDrill().catch((error) => {
  console.error(`[publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
