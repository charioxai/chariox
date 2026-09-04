// Failure evidence for the owned drill slice, before lifecycle cleanup erases
// the process record. Never copy raw streamer logs or private token records.
const probe = `
import json, sys
sys.path.insert(0, '/opt/chariox-slice')
from selkies_viewers import lifecycle
directory = lifecycle.state_directory()
record = lifecycle.read_state(directory)
process = lifecycle.owned_process(record)
result = {'recorded': record is not None, 'owned': process is not None,
          'healthy': lifecycle.healthy(record)}
if process is not None:
    result.update(pid=process.pid, processStatus=process.status())
try:
    with (directory / 'streamer.log').open('rb') as stream:
        stream.seek(0, 2)
        size = stream.tell()
        stream.seek(max(0, size - 65536))
        text = stream.read(65536).decode(errors='replace')
    result['logBytes'] = size
    result['signals'] = [word for word in ['Traceback', 'Connection refused', 'Address already in use',
        'No space left', 'Cannot open display', 'Segmentation fault', 'Broken pipe', 'ERROR', 'CRITICAL'] if word in text]
    result['exceptionTypes'] = [name for name in ['AssertionError', 'RuntimeError', 'TimeoutError',
        'OSError', 'PermissionError', 'BrokenPipeError', 'ConnectionResetError', 'ModuleNotFoundError'] if name in text]
except FileNotFoundError:
    result['logMissing'] = True
print(json.dumps(result))
`

export async function captureRoomStreamerDiagnostics(containerName, runCommand) {
  if (!/^chariox-slice-room-pointer-\d+-/.test(containerName)) {
    throw new Error("streamer diagnostic requires a drill-owned slice")
  }
  const result = await runCommand("docker", ["exec", "-u", "slice", containerName,
    "timeout", "--kill-after=1s", "5s", "/opt/chariox-selkies/bin/python", "-c", probe], 8_000)
  if (result.code !== 0) return { status: "unavailable", exitCode: result.code }
  try {
    return { status: "captured", ...JSON.parse(result.stdout) }
  } catch {
    return { status: "unavailable", reason: "invalid-diagnostic-json" }
  }
}
