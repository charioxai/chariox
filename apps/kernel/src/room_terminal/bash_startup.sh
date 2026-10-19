# Run once, at the first prompt of an interactive login bash. No user file is edited.
unset PROMPT_COMMAND PS0
[[ -r /etc/profile ]] && source /etc/profile
for __chariox_profile in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
  if [[ -r "$__chariox_profile" ]]; then
    source "$__chariox_profile"
    break
  fi
done
unset __chariox_profile

# A scalar wrapper works on system Bash 3.2 as well as modern Bash. Keep
# every profile-installed hook and the status each preceding hook returned.
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
  __chariox_prompt_hooks=("${PROMPT_COMMAND[@]}")
else
  __chariox_prompt_hooks=("${PROMPT_COMMAND-}")
fi
__chariox_debug_start=0
__chariox_status() { return "$1"; }
__chariox_prompt() {
  local __chariox_status_code=$? __chariox_hook
  # Old Bash skips DEBUG for top-level subshell syntax. Finish those lines
  # with a paired mark; the PTY tracks their foreground jobs while they run.
  if [[ $__chariox_debug_start == 1 && $__chariox_at_prompt == 1 ]]; then
    printf '\033]133;C\007'
  fi
  printf '\033]133;D;%s\007' "$__chariox_status_code"
  for __chariox_hook in "${__chariox_prompt_hooks[@]}"; do
    __chariox_status "$__chariox_status_code"
    eval "$__chariox_hook"
    __chariox_status_code=$?
  done
  __chariox_at_prompt=1
  return "$__chariox_status_code"
}
unset PROMPT_COMMAND
PROMPT_COMMAND=__chariox_prompt

if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
  PS0='\e]133;C\a'"${PS0-}"
else
  # Bash before 4.4 has no PS0. DEBUG runs before even a blocking builtin;
  # emit once per command line, excluding prompt hooks and function bodies.
  # Subshells can outlive the prompt, so only the parent shell reports starts.
  __chariox_debug_start=1
  __chariox_at_prompt=0
  __chariox_started() {
    local __chariox_status_code=$?
    if [[ $__chariox_at_prompt == 1 && $BASH_SUBSHELL == 0 && $1 != __chariox_prompt && ${#FUNCNAME[@]} == 1 ]]; then
      __chariox_at_prompt=0
      printf '\033]133;C\007'
    fi
    return "$__chariox_status_code"
  }
  __chariox_debug=$(trap -p DEBUG)
  if [[ -n $__chariox_debug ]]; then
    __chariox_debug=${__chariox_debug#trap -- }
    __chariox_debug=${__chariox_debug% DEBUG}
    eval "__chariox_debug=$__chariox_debug"
  fi
  trap '__chariox_started "$BASH_COMMAND"; eval "$__chariox_debug"' DEBUG
fi
# Nested shells must install their own integration.
export -n PROMPT_COMMAND PS0
# Bash already expanded the original one-shot hook: run this first prompt too.
__chariox_prompt
