# Shell entry points pass their PID explicitly because command substitution adds a process.
drill_docker_environment() {
  local mode="$1" harness="$2" assignment assignments
  assignments="$(node "$(dirname "${BASH_SOURCE[0]}")/drill-docker-owner.mjs" "$mode" "$harness" "$$")" || return
  while IFS= read -r assignment; do
    export "$assignment"
  done <<< "$assignments"
}
