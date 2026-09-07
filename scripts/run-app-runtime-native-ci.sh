#!/usr/bin/env bash
# Destructive SDK cleanup is restricted to the disposable hosted Linux runner.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != true || "${RUNNER_ENVIRONMENT:-}" != github-hosted || "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo 'Native CI builds require a disposable GitHub-hosted Linux x64 runner.' >&2
  exit 1
fi
if [[ ! "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ || ! "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ ]]; then
  echo 'Missing hosted run identity.' >&2
  exit 1
fi

ci_workspace="$(realpath -e -- "$GITHUB_WORKSPACE")"
ci_temp="$(realpath -e -- "$RUNNER_TEMP")"
cd "$ci_workspace"
ci_image="$(node -p "JSON.parse(require('fs').readFileSync('apps/app-worker/runtime.lock.json', 'utf8')).dedicatedCi.builderImage")"
if [[ ! "$ci_image" =~ ^node:[0-9]+\.[0-9]+\.[0-9]+-bookworm@sha256:[a-f0-9]{64}$ ]]; then
  echo 'A digest-pinned official Debian Node builder is required.' >&2
  exit 1
fi

# These preinstalled SDKs are unrelated to this isolated C/C++ build. The VM is
# discarded after this job; no developer machine or self-hosted runner enters.
df -PB1 "$ci_temp"
sudo rm -rf -- /usr/local/lib/android /usr/share/dotnet /opt/ghc /usr/local/.ghcup
df -PB1 "$ci_temp"
docker pull "$ci_image"

ci_scratch="$(mktemp -d "$ci_temp/chariox-native.XXXXXXXX")"
ci_container="chariox-native-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
mkdir -m 700 -- "$ci_scratch/home"
printf 'scratch=%s\nartifact_path=%s/runtime/artifacts\n' "$ci_scratch" "$ci_scratch" >> "$GITHUB_OUTPUT"
cleanup_container() { docker container rm -f "$ci_container" >/dev/null 2>&1 || true; }
trap cleanup_container EXIT
trap 'exit 143' TERM
trap 'exit 130' INT

# Only the CI markers and an empty scratch home cross into the container.
# Its cgroup settings are independently checked by the Node build driver.
timeout --signal=TERM --kill-after=10s 165m docker run --rm --init \
  --name "$ci_container" --cpus=2 --memory=6g --memory-swap=6g --pids-limit=256 \
  --cap-drop=ALL --security-opt=no-new-privileges --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=256m \
  --user "$(id -u):$(id -g)" \
  --env GITHUB_ACTIONS=true --env RUNNER_ENVIRONMENT=github-hosted --env HOME=/build-scratch/home \
  --mount "type=bind,source=$ci_workspace,target=/workspace,readonly" \
  --mount "type=bind,source=$ci_scratch,target=/build-scratch" \
  --workdir /workspace --entrypoint /usr/local/bin/node "$ci_image" \
  scripts/build-app-runtime.mjs build --target linux-x64 --jobs 1 \
  --resource-profile github-linux --scratch /build-scratch/runtime --download-source
