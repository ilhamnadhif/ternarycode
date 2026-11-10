# TernaryCode shell integration for zsh: OSC 133 prompt marks (A prompt start, B prompt end,
# C command start, D;status command end) and the working directory (OSC 7).
#
# Sourced by .zshenv in interactive shells, inside a function with `emulate -L zsh -o no_aliases`
# (so globals need `typeset -g`). Nothing is installed now: a one-shot precmd hook does it at the
# first prompt, after the user's .zshrc and .zlogin, so that their hooks and theme come first.
#
# Hooks:
# - _tc_precmd, first precmd hook: D;$? if a command ran, then OSC 7 and A. Every precmd hook
#   sees the command's $?, and hook return values never change it.
# - _tc_precmd_ps1, last precmd hook: appends B (inside %{...%}) to PS1, after themes that set
#   PS1 in their own precmd hooks; it moves itself back to the end when something else was added
#   after it.
# - _tc_preexec: C. Also restores the unmarked PS1, so that commands see the user's own prompt.

(( ${+_tc_state} )) && builtin return 0  # already loaded

# 1 while a command runs: C was written and D is due.
builtin typeset -gi _tc_state=0
builtin typeset -g _tc_ps1_saved= _tc_ps1_marked= _tc_cwd= _tc_cwd_url=

# Percent-encodes $1 byte by byte for a file:// URL (unreserved characters and / stay); sets REPLY.
_tc_url_path() {
    builtin emulate -L zsh -o no_multibyte
    if [[ $1 != *[^A-Za-z0-9/._~-]* ]]; then
        REPLY=$1
        builtin return 0
    fi
    REPLY=
    builtin local c
    builtin local -i i
    for (( i = 1; i <= $#1; i++ )); do
        c=${1[i]}
        if [[ $c == [A-Za-z0-9/._~-] ]]; then
            REPLY+=$c
        else
            REPLY+=%${(l:2::0:)$(( [##16] #c ))}
        fi
    done
}

_tc_precmd() {
    builtin local -i s=$?
    builtin emulate -L zsh
    # Some plugins run precmd hooks from a widget: the cursor is in the command line then.
    builtin zle 2>/dev/null && builtin return 0
    if (( _tc_state == 1 )); then
        builtin print -rn -- $'\e]133;D;'$s$'\a'
        _tc_state=0
    fi
    if [[ $PWD != "$_tc_cwd" ]]; then
        builtin local REPLY
        _tc_url_path "$PWD"
        _tc_cwd=$PWD
        _tc_cwd_url=$REPLY
    fi
    # The prompt start, also put into PS1 (below) so it comes again with every redraw: here it
    # still marks a first prompt whose PS1 a later hook replaced. zsh redraws the whole prompt
    # after a resize (redraw=1): TernaryCode clears it first.
    builtin print -rn -- $'\e]7;file://'"$HOST$_tc_cwd_url"$'\a\e]133;A;redraw=1\a'
}

_tc_precmd_ps1() {
    # The user's own option, read before `emulate` sets it for this function.
    builtin local _tc_percent=0
    [[ -o prompt_percent ]] && _tc_percent=1
    builtin emulate -L zsh
    if [[ ${precmd_functions[-1]-} != _tc_precmd_ps1 ]]; then
        precmd_functions=(${precmd_functions:#_tc_precmd_ps1} _tc_precmd_ps1)
    fi
    (( _tc_percent )) || builtin return 0
    # Start from the unmarked prompt unless something replaced it since.
    if [[ -n $_tc_ps1_marked && $PS1 == "$_tc_ps1_marked" ]]; then
        PS1=$_tc_ps1_saved
    fi
    _tc_ps1_saved=$PS1
    # A lone trailing % would swallow the % of %{.
    [[ $PS1 == *[^%]% || $PS1 == % ]] && PS1+=%
    PS1=$'%{\e]133;A;redraw=1\a%}'$PS1$'%{\e]133;B\a%}'
    _tc_ps1_marked=$PS1
}

_tc_preexec() {
    builtin emulate -L zsh
    if [[ -n $_tc_ps1_marked && $PS1 == "$_tc_ps1_marked" ]]; then
        PS1=$_tc_ps1_saved
    fi
    builtin print -rn -- $'\e]133;C\a'
    _tc_state=1
}

_tc_deferred_init() {
    builtin emulate -L zsh
    precmd_functions=(_tc_precmd ${precmd_functions:#_tc_deferred_init})
    if builtin autoload -Uz +X add-zsh-hook 2>/dev/null; then
        add-zsh-hook preexec _tc_preexec
        add-zsh-hook precmd _tc_precmd_ps1
    else
        builtin typeset -ga preexec_functions
        preexec_functions+=(_tc_preexec)
        precmd_functions+=(_tc_precmd_ps1)
    fi
    builtin unfunction _tc_deferred_init
    # This prompt's round of hooks is already running without them.
    _tc_precmd
    _tc_precmd_ps1
}

builtin typeset -ga precmd_functions
precmd_functions+=(_tc_deferred_init)
