import path from 'node:path'
import { mkdir, writeFile } from 'node:fs/promises'
import { freePort, logStep, repoRoot, startProcess, stopProcess } from './live-workflow-publication-drill-runtime.mjs'
import { waitForGateway, waitForRegisteredPublicationEndpoint } from './live-workflow-publication-drill-waiters.mjs'

export async function runLivePublicationManifestMode({
  manifestPath,
  client,
  env,
  kernelUrl,
  relayPort,
  publications,
}) {
  const gateways = []
  try {
    const manifest = {
      generated_at_ms: Date.now(),
      relay_display_prefix: `http://127.0.0.1:${relayPort}/display/`,
      publications: {},
    }
    for (const item of publications) {
      const port = await freePort()
      const localUrl = `http://127.0.0.1:${port}`
      const gateway = startProcess(
        process.execPath,
        [path.join(repoRoot, 'apps/server/dist/index.js')],
        {
          ...env,
          HOST: '127.0.0.1',
          PORT: String(port),
          ARROBA_KERNEL_URL: kernelUrl,
          ARROBA_PUBLICATION_SESSION_ID: item.sessionId,
          ARROBA_PUBLICATION_ID: item.publication.id,
        },
        `gateway-live-${item.key}`,
      )
      gateways.push(gateway)
      await waitForGateway(localUrl)
      const registered = await waitForRegisteredPublicationEndpoint(
        client,
        item.sessionId,
        item.publication.id,
        `${localUrl}/`,
        `http://127.0.0.1:${relayPort}/display/publication-`,
      )
      manifest.publications[item.key] = {
        id: item.publication.id,
        alias: item.publication.alias ?? null,
        route: item.publication.route ?? null,
        transport: item.transport,
        local_url: localUrl,
        open_url: registered.open_url,
        session_id: item.sessionId,
        expected_html_text: item.expectedHtmlText ?? null,
        required_html_snippets: item.requiredHtmlSnippets ?? null,
        prompt_text: item.promptText ?? null,
        required_trace_levels: item.requiredTraceLevels ?? null,
        required_trace_alias: item.requiredTraceAlias ?? null,
      }
    }
    await mkdir(path.dirname(manifestPath), { recursive: true })
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
    logStep('live_publication_manifest_ready', { manifestPath, publications: Object.keys(manifest.publications) })
    await waitForStopSignal()
  } finally {
    await Promise.all(gateways.map((gateway) => stopProcess(gateway).catch(() => {})))
  }
}

export async function waitForStopSignal() {
  await new Promise((resolve) => {
    const done = () => {
      process.off('SIGTERM', done)
      process.off('SIGINT', done)
      resolve()
    }
    process.once('SIGTERM', done)
    process.once('SIGINT', done)
  })
}

export { isTerminalWorkflowRunStatus } from './live-workflow-publication-drill-runtime.mjs'
