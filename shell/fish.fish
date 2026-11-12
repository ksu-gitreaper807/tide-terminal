if set -q __tide_loaded
    return
end
set -g __tide_loaded 1
function __tide_emit
    set -l payload ''
    if test (count $argv) -gt 1
        set payload $argv[2]
    end
    printf '\033]777;tide;v1;%s;%s;%s\007' "$TIDE_MARKER_TOKEN" "$argv[1]" "$payload"
    if test "$TIDE_SPLIT_CAPTURE" = 1
        printf '\033]777;tide;v1;%s;%s;%s\007' "$TIDE_MARKER_TOKEN" "$argv[1]" "$payload" >&2
    end
end
function T_:
    "$TIDE_EXE" timer $argv
end
function A_:
    __tide_emit append
    "$TIDE_EXE" append $argv
end
function __tide_preexec --on-event fish_preexec
    if not string match -qr '^ ' -- $argv[1]
        __tide_emit command (printf '%s' "$argv[1]" | base64 | string join '')
    end
    __tide_emit start
end
function __tide_postexec --on-event fish_postexec
    set -l last_status $status
    __tide_emit end $last_status
end
function __tide_prompt --on-event fish_prompt
    __tide_emit prompt
    __tide_emit cwd (printf '%s' "$PWD" | base64 | string join '')
end
function __tide_lookup
    set -l result ("$TIDE_EXE" history --query (commandline) --cwd "$PWD" --first | string collect)
    if test -n "$result"
        commandline --replace -- "$result"
    end
    commandline -f repaint
end
bind \cx\ch __tide_lookup
function __tide_insert
    set -l selected (string collect < "$TIDE_SELECTION_FILE")
    commandline --replace -- "$selected"
    commandline -f repaint
end
bind \cx\cy __tide_insert
