# TernaryCode shell integration for bash (3.2 and later): OSC 133 prompt marks (A prompt start,
# B prompt end, C command start, D;status command end) and the working directory (OSC 7).
#
# TernaryCode puts this bootstrap in front of PROMPT_COMMAND in the environment:
#   . '<this file>';builtin eval "${__tc_late-}"
# bash reads its startup files itself, as usual; the bootstrap runs at the first prompt, after all
# of them. This file then puts its hooks around whatever PROMPT_COMMAND the user's files left:
#   __tc_precmd "$?"      first: D;status if a command ran, then OSC 7 and A
#   ...the user's PROMPT_COMMAND...
#   __tc_prompt_end "$?"  last: appends B to PS1 (after themes that set PS1 in PROMPT_COMMAND)
# and the `eval` installs a DEBUG trap that writes C before the first command of a command line
# (a DEBUG trap that was already set keeps running). The trap has to be read and set outside any
# function or sourced file: bash puts back the DEBUG trap it had when those return.
# Every hook hands $? on unchanged.
#
# (bash 3.2 ignores ENV even in posix mode, so the `--posix` + ENV route, which would not depend
# on PROMPT_COMMAND surviving the user's files, is not available through the environment alone.)

__tc_s=$?
if [[ $- != *i* ]]; then
    builtin return $__tc_s 2>/dev/null || builtin exit $__tc_s
fi
# Loaded already (the bootstrap could not be taken out of PROMPT_COMMAND): __tc_late only hands
# the status on now.
if [[ -n ${__tc_loaded-} ]]; then
    builtin return $__tc_s
fi
__tc_loaded=1

# The environment exported PROMPT_COMMAND; programs started from here need not see it.
builtin export -n PROMPT_COMMAND 2>/dev/null

# 0: nothing due; 1: at the prompt, C due; 2: C written, D due; 3: like 1, but the rest of the
# first prompt's PROMPT_COMMAND (whose commands are in __tc_first) is still to run.
__tc_state=0
__tc_first=
__tc_prev_debug=
__tc_ps1_saved=
__tc_ps1_marked=
__tc_cwd=
__tc_cwd_url=

__tc_ret() {
    builtin return ${1:-0}
}

__tc_report_cwd() {
    if [[ $PWD != "$__tc_cwd" ]]; then
        # Byte by byte, and ranges by byte value: the C locale.
        builtin local LC_ALL=C i c h u=
        if [[ $PWD == *[!A-Za-z0-9/._~-]* ]]; then
            for ((i = 0; i < ${#PWD}; i++)); do
                c=${PWD:i:1}
                if [[ $c == [A-Za-z0-9/._~-] ]]; then
                    u+=$c
                else
                    builtin printf -v h '%02X' "'$c"
                    # Bytes over 127 come out sign-extended (FFFFFFC3): keep the last two digits.
                    u+=%${h: -2:2}
                fi
            done
        else
            u=$PWD
        fi
        __tc_cwd=$PWD
        __tc_cwd_url=$u
    fi
    builtin printf '\033]7;file://%s%s\007' "${HOSTNAME-}" "$__tc_cwd_url"
}

__tc_precmd() {
    if [[ $__tc_state == 2 ]]; then
        builtin printf '\033]133;D;%s\007' "$1"
    fi
    __tc_state=0
    __tc_report_cwd
    # readline redraws only the prompt's last line after a resize.
    builtin printf '\033]133;A;redraw=last\007'
    builtin return ${1:-0}
}

__tc_prompt_end() {
    # Start from the unmarked prompt unless something replaced it since.
    if [[ -n $__tc_ps1_marked && $PS1 == "$__tc_ps1_marked" ]]; then
        PS1=$__tc_ps1_saved
    fi
    __tc_ps1_saved=$PS1
    PS1=$PS1'\[\033]133;B\007\]'
    __tc_ps1_marked=$PS1
    __tc_state=1
    builtin return ${1:-0}
}

# Shell code as a list of words: separators become spaces, one space between words, one around.
__tc_words() {
    builtin local s="$1"
    s=${s//$'\n'/ }
    s=${s//$'\t'/ }
    s=${s//;/ }
    s=${s//&/ }
    s=${s//|/ }
    s=${s//\(/ }
    s=${s//\)/ }
    while [[ $s == *'  '* ]]; do
        s=${s//  / }
    done
    s=${s# }
    __tc_out=" ${s% } "
}

# The DEBUG trap: $1 is $_, and being the last argument here keeps $_ for the command.
__tc_debug() {
    # The first command after the prompt: at the top level (not inside a function that has the
    # trace attribute, or with `set -T`; bash-preexec calls this trap as its preexec function
    # __bp_original_debug_trap, for commands only), and not ours (an empty command line runs
    # PROMPT_COMMAND directly), a completion function or a `bind -x` widget.
    if [[ ($__tc_state == 1 || $__tc_state == 3) &&
        (${#FUNCNAME[@]} == 1 || ${FUNCNAME[1]-} == __bp_original_debug_trap) &&
        ${BASH_SUBSHELL-0} == 0 && -z ${COMP_LINE-} && -z ${READLINE_LINE+x} &&
        $BASH_COMMAND != __tc_* ]]; then
        # In state 3, commands of the first prompt's PROMPT_COMMAND do not count.
        if [[ $__tc_state == 1 ]] || {
            __tc_words "$BASH_COMMAND"
            [[ $__tc_first != *"$__tc_out"* ]]
        }; then
            __tc_state=2
            if [[ -n $__tc_ps1_marked && $PS1 == "$__tc_ps1_marked" ]]; then
                PS1=$__tc_ps1_saved
            fi
            builtin printf '\033]133;C\007'
        fi
    fi
    if [[ -n $__tc_prev_debug ]]; then
        : "$1"
        builtin eval "$__tc_prev_debug"
    else
        # With `shopt -s extdebug` a non-zero status would skip the command.
        builtin return 0
    fi
}

__tc_boot=". '${BASH_SOURCE[0]}';builtin eval \"\${__tc_late-}\""

# Takes the bootstrap out of one PROMPT_COMMAND entry, when it sits at the start or the end; the
# result goes to __tc_out. Anywhere else it stays: running it again only hands the status on.
__tc_unboot() {
    builtin local rest
    __tc_out=$1
    case $1 in
        "$__tc_boot")
            __tc_out=
            ;;
        "$__tc_boot"*)
            rest=${1#"$__tc_boot"}
            rest=${rest#"${rest%%[![:blank:]]*}"}
            case $rest in
                ';'* | $'\n'*) rest=${rest#?} ;;
                *) builtin return 0 ;;
            esac
            rest=${rest#"${rest%%[![:blank:]]*}"}
            case $rest in
                ';'* | '&'* | '|'*) ;;
                *) __tc_out=$rest ;;
            esac
            ;;
        *"$__tc_boot")
            rest=${1%"$__tc_boot"}
            case ${rest%"${rest##*[![:blank:]]}"} in
                '' | *';' | *$'\n') __tc_out=$rest ;;
            esac
            ;;
    esac
}

# Install. PROMPT_COMMAND can be an array from bash 5.1 on; every entry runs then.
if (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) )) &&
    [[ $(builtin declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
    __tc_e=$IFS
    IFS=$'\n'
    __tc_first=${PROMPT_COMMAND[*]}
    IFS=$__tc_e
    __tc_pc=('__tc_precmd "$?"')
    for __tc_e in "${PROMPT_COMMAND[@]}"; do
        __tc_unboot "$__tc_e"
        [[ -z $__tc_out ]] || __tc_pc+=("$__tc_out")
    done
    __tc_pc+=('__tc_prompt_end "$?"')
    PROMPT_COMMAND=("${__tc_pc[@]}")
    builtin unset __tc_pc __tc_e
else
    __tc_first=${PROMPT_COMMAND-}
    __tc_unboot "$__tc_first"
    if [[ -n $__tc_out ]]; then
        __tc_out=$__tc_out$'\n'
    fi
    PROMPT_COMMAND='__tc_precmd "$?"'$'\n'$__tc_out'__tc_prompt_end "$?"'
fi

# This prompt's PROMPT_COMMAND is already running without the hooks.
__tc_precmd "$__tc_s"
__tc_prompt_end "$__tc_s"

# What this prompt's PROMPT_COMMAND still runs after the bootstrap.
__tc_first=${__tc_first#*"$__tc_boot"}
__tc_words "$__tc_first"
if [[ $__tc_out == ' ' ]]; then
    __tc_first=
else
    __tc_first=$__tc_out
    __tc_state=3
fi
builtin unset __tc_out

# Run by the bootstrap right after this file, outside of it.
__tc_late='builtin eval "__tc_terms=( $(builtin trap -p DEBUG) )"
__tc_prev_debug=${__tc_terms[2]-}
builtin unset __tc_terms
builtin trap -- '\''__tc_debug "$_"'\'' DEBUG
__tc_late='\''__tc_ret "$__tc_s"'\''
__tc_ret "$__tc_s"'

builtin return $__tc_s
