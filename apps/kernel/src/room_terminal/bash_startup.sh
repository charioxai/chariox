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

__chariox_finished() {
  local status_code=$?
  printf '\033]133;D;%s\007' "$status_code"
  return "$status_code"
}
# Preserve both scalar and array prompt hooks, including the command's status.
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a '* ]]; then
  PROMPT_COMMAND=(__chariox_finished "${PROMPT_COMMAND[@]}")
else
  PROMPT_COMMAND=(__chariox_finished "${PROMPT_COMMAND-}")
fi
PS0='\e]133;C\a'"${PS0-}"
# Nested shells must install their own integration.
export -n PROMPT_COMMAND PS0
# Bash already expanded the original one-shot hook. Run the composed hooks
# for this first prompt too, so profile-built prompts are ready immediately.
for __chariox_prompt in "${PROMPT_COMMAND[@]}"; do
  eval "$__chariox_prompt"
done
unset __chariox_prompt
