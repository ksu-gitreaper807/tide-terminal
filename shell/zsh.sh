[[ ${__tide_loaded-} == 1 ]] && return
typeset -g __tide_loaded=1
# Shell job control must initialize against the controlling terminal first.
# Only subsequent commands inherit the separate stderr output terminal.
if [[ ${TIDE_SPLIT_CAPTURE-} == 1 ]]; then exec 2>"$TIDE_STDERR_TTY"; fi
# Mirror capture boundaries to both terminal channels. Bash PS0 runs inside
# command substitution, so retain the original stdout for its control markers.
__tide_emit() {
    printf '\033]777;tide;v1;%s;%s;%s\007' "$TIDE_MARKER_TOKEN" "$1" "${2-}"
    if [[ ${TIDE_SPLIT_CAPTURE-} == 1 ]]; then printf '\033]777;tide;v1;%s;%s;%s\007' "$TIDE_MARKER_TOKEN" "$1" "${2-}" >&2; fi
}
T_:() { "$TIDE_EXE" timer "$@"; }
A_:() {
    __tide_emit append
    "$TIDE_EXE" append "$@"
}
# Zsh emits PROMPT_SP decoration before precmd. End capture ahead of that output;
# retain the user's visual marker and leave the precmd fallback for PROMPT_SP off.
typeset -g PROMPT_EOL_MARK=$'%{\e]777;tide;v1;'"$TIDE_MARKER_TOKEN"$';end;%?\a%}'"${PROMPT_EOL_MARK-'%B%S%#%s%b'}"
autoload -Uz add-zsh-hook
__tide_preexec() {
    # Respect HIST_IGNORE_SPACE and always omit a leading-space command from app history.
    if [[ $1 != ' '* ]]; then
        __tide_emit command "$(printf '%s' "$1" | base64 | tr -d '\n')"
    fi
    __tide_emit start
}
__tide_precmd() {
    local __tide_status=$?
    __tide_emit end "$__tide_status"
    __tide_emit prompt
    __tide_emit cwd "$(printf '%s' "$PWD" | base64 | tr -d '\n')"
    return "$__tide_status"
}
add-zsh-hook preexec __tide_preexec
add-zsh-hook precmd __tide_precmd
__tide_lookup() {
    local result
    result=$("$TIDE_EXE" history --query "$BUFFER" --cwd "$PWD" --first)
    if [[ -n $result ]]; then BUFFER=$result; CURSOR=${#BUFFER}; fi
    zle redisplay
}
zle -N __tide_lookup
bindkey '^X^H' __tide_lookup
__tide_insert() {
    BUFFER=$(cat -- "$TIDE_SELECTION_FILE")
    CURSOR=${#BUFFER}
    zle redisplay
}
zle -N __tide_insert
bindkey '^X^Y' __tide_insert
