// Failure evidence for the owned drill slice, before lifecycle cleanup erases
// the process record. Never copy raw streamer logs or private token records.
const probe = `
import json, sys
from pathlib import Path
import psutil
sys.path.insert(0, '/opt/chariox-slice')
from selkies_viewers import lifecycle
directory = lifecycle.state_directory()
record = lifecycle.read_state(directory)
process = lifecycle.owned_process(record)
result = {'recorded': record is not None, 'owned': process is not None,
          'healthy': lifecycle.healthy(record)}
if record is not None:
    try:
        candidate = psutil.Process(record['pid'])
        result['recordedProcess'] = {
            'exists': True,
            'sameCreationTime': candidate.create_time() == record['created'],
            'sameUser': candidate.uids().real == lifecycle.os.getuid(),
            'status': candidate.status(),
        }
    except psutil.NoSuchProcess:
        result['recordedProcess'] = {'exists': False}
    except psutil.AccessDenied:
        result['recordedProcess'] = {'accessDenied': True}
# cgroup counters survive a killed process, unlike instantaneous docker stats.
# Only numeric allowlisted fields are retained; no environment or command line.
cgroup = Path('/sys/fs/cgroup')
result['cgroup'] = {}
for name in ['memory.current', 'memory.peak', 'memory.max', 'pids.current', 'pids.max']:
    try:
        value = (cgroup / name).read_text()[:128].strip()
        if value == 'max' or value.isdecimal():
            result['cgroup'][name] = value if value == 'max' else int(value)
    except OSError:
        pass
try:
    values = {}
    for line in (cgroup / 'memory.events').read_text()[:4096].splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[0] in ['low', 'high', 'max', 'oom', 'oom_kill', 'oom_group_kill'] and parts[1].isdecimal():
            values[parts[0]] = int(parts[1])
    result['cgroup']['memory.events'] = values
except OSError:
    pass
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
