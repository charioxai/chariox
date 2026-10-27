"""MP-08/MP-10/MP-11: read protocol pins from the exact bound source blobs."""
import re

PROTOCOL_FILES = {
 'protocol': ('packages/kernel-client/src/kernel-types.ts', r'LOCAL_DAEMON_PROTOCOL_VERSION\s*=\s*(\d+)\b'),
 'relay': ('apps/kernel/src/transport/relay_peer.rs', r'RELAY_PEER_PROTOCOL_VERSION:\s*u32\s*=\s*(\d+)\b'),
}

def versions(read_source):
 result = {}
 for name, (path, pattern) in PROTOCOL_FILES.items():
  matches = re.findall(pattern, read_source(path).decode())
  if len(matches) != 1:raise ValueError('MP-10: ambiguous source protocol pin '+name)
  result[name] = int(matches[0])
 return result
