#!/bin/bash
# MP-08/MP-10/MP-11: exact recorded PIDs/start identities only.
set -euo pipefail
KIT=$(cd -- "$(dirname -- "$0")" && pwd)
: "${AB_STATE:?same disposable state path used by ab-up required}"
export AB_KIT="$KIT"
python3 "$KIT/ab-processes.py" stop
