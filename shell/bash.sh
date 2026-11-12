# Loaded after the user's .bashrc. Preserve existing prompt hooks and PS0.
[[ ${__tide_loaded-} == 1 ]] && return
__tide_loaded=1
# Separate process: preserve quoting and return the supervised command's exit status.
# Shell job control must initialize against the controlling terminal first.
# Only subsequent commands inherit the separate stderr output terminal.
if [[ ${TIDE_SPLIT_CAPTURE-} == 1 ]]; then exec 2>"$TIDE_STDERR_TTY"; fi
# Mirror capture boundaries to both terminal channels. Bash PS0 runs inside
# command substitution, so retain the original stdout for its control markers.
if [[ ${TIDE_SPLIT_CAPTURE-} == 1 ]]; then exec {__tide_emit_fd}>&1; fi
__tide_emit() {
    if [[ ${TIDE_SPLIT_CAPTURE-} == 1 ]]; then
        printf "$@" >&"$__tide_emit_fd"
        printf "$@" >&2
    else
        printf "$@"
    fi
}
T_:() { "$TIDE_EXE" timer "$@"; }
A_:() {
    __tide_emit '\033]777;tide;append\007'
    "$TIDE_EXE" append "$@"
}
__tide_before=''
__tide_prompt() {
    local __tide_status=$?
    __tide_emit '\033]133;D;%s\007\033]133;A\007' "$__tide_status"
    __tide_emit '\033]777;tide;cwd;%s\007' "$(printf '%s' "$PWD" | base64 | tr -d '\n')"
    __tide_before=$(HISTTIMEFORMAT= builtin history 1)
    return "$__tide_status"
}
__tide_preexec() {
    local __tide_current __tide_cmd
    __tide_current=$(HISTTIMEFORMAT= builtin history 1)
    if [[ $__tide_current != "$__tide_before" && -n $__tide_current ]]; then
        __tide_cmd=${__tide_current#*[0-9]  }
        __tide_emit '\033]777;tide;command;%s\007' "$(printf '%s' "$__tide_cmd" | base64 | tr -d '\n')"
    fi
    __tide_emit '\033]133;C\007'
}
if declare -F blehook >/dev/null; then
    __tide_ble_preexec() {
        if [[ $1 != ' '* ]]; then
            __tide_emit '\033]777;tide;command;%s\007' "$(printf '%s' "$1" | base64 | tr -d '\n')"
        fi
        __tide_emit '\033]133;C\007'
    }
    blehook PREEXEC+=__tide_ble_preexec
    blehook PRECMD+=__tide_prompt
elif [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
    PROMPT_COMMAND=(__tide_prompt "${PROMPT_COMMAND[@]}")
else
    PROMPT_COMMAND="__tide_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi
if ! declare -F blehook >/dev/null; then PS0='$(__tide_preexec)'${PS0-}; fi
__tide_lookup() {
    local __tide_result
    __tide_result=$("$TIDE_EXE" history --query "$READLINE_LINE" --cwd "$PWD" --first)
    if [[ -n $__tide_result ]]; then local LC_ALL=C; READLINE_LINE=$__tide_result; READLINE_POINT=${#READLINE_LINE}; fi
}
bind -x '"\C-x\C-h":__tide_lookup'
__tide_insert() {
    local LC_ALL=C
    READLINE_LINE=$(cat -- "$TIDE_SELECTION_FILE")
    READLINE_POINT=${#READLINE_LINE}
}
bind -x '"\C-x\C-y":__tide_insert'
