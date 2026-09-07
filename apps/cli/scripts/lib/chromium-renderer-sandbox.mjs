import assert from "node:assert/strict"

// Read only process security metadata, never URLs, page text, or cookie values.
const probe = `
import glob, json, os
records=[]
for path in glob.glob('/proc/[0-9]*'):
    try:
        # Chromium can rewrite its process title into one space-separated argv entry.
        argv=open(path+'/cmdline','rb').read().replace(b'\\0',b' ').split()
        if not argv or os.path.basename(argv[0]) not in (b'chromium',b'chrome'):
            continue
        renderer=b'--type=renderer' in argv
        if not renderer and any(a.startswith(b'--type=') for a in argv):
            continue
        fields={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in open(path+'/status') if ':' in line}
        records.append({'renderer':renderer, 'uid':int(fields['Uid'].split()[0]),
            'capabilities':int(fields['CapEff'],16), 'noNewPrivileges':int(fields['NoNewPrivs']),
            'seccomp':int(fields['Seccomp']), 'filters':int(fields['Seccomp_filters']),
            'pidNamespaceDepth':len(fields['NSpid'].split())})
    except (FileNotFoundError,ProcessLookupError):
        pass
print(json.dumps(records))
`

export async function verifyChromiumRendererSandbox(run) {
  const records = JSON.parse(await run(["python3", "-c", probe]))
  const browsers = records.filter(record => !record.renderer)
  const renderers = records.filter(record => record.renderer)
  assert.equal(browsers.length, 1, "expected one Chromium browser in the disposable drill")
  assert.ok(renderers.length > 0, "no Chromium renderer available for sandbox verification")
  for (const renderer of renderers) {
    assert.ok(renderer.uid > 0, "renderer must run without root identity")
    assert.equal(renderer.capabilities, 0, "renderer retained capabilities")
    assert.equal(renderer.noNewPrivileges, 1, "renderer allows new privileges")
    assert.equal(renderer.seccomp, 2, "renderer lacks seccomp filtering")
    assert.ok(renderer.filters > browsers[0].filters, "renderer lacks its own seccomp filter")
    assert.ok(renderer.pidNamespaceDepth > browsers[0].pidNamespaceDepth,
      "renderer lacks its own PID namespace")
  }
  return { rendererCount: renderers.length, rendererSandboxVerified: true }
}
