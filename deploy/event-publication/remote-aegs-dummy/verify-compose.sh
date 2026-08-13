#!/usr/bin/env bash
set -euo pipefail

deployment_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary_dir="$(mktemp -d)"
trap 'rm -rf "${temporary_dir}"' EXIT

touch "${temporary_dir}/producer-token" "${temporary_dir}/management-token"
export CHARIOX_AEGS_DUMMY_IMAGE="sha256:$(printf '0%.0s' {1..64})"
export CHARIOX_AEDS_EVENTS_URL="https://aeds.example.test/v1/events"
export CHARIOX_AEGS_PRODUCER_TOKEN_FILE="${temporary_dir}/producer-token"
export CHARIOX_AEGS_MANAGEMENT_TOKEN_FILE="${temporary_dir}/management-token"

rendered="$(docker compose -f "${deployment_dir}/compose.yaml" config --format json)"
jq -e '.services["dummy-aegs"].tmpfs == ["/tmp:size=16m,mode=1777"]' \
  <<<"${rendered}" >/dev/null
