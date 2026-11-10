# TernaryCode shell integration for zsh: the first file zsh reads, found through ZDOTDIR.
#
# TernaryCode starts zsh with ZDOTDIR pointing at this directory and the user's own ZDOTDIR (when
# there is one) in TERNARYCODE_ZSH_ZDOTDIR. This file puts ZDOTDIR back first and then sources
# the user's .zshenv, so zsh goes on to read /etc/zprofile, .zprofile, /etc/zshrc, .zshrc,
# /etc/zlogin and .zlogin from the user's directory exactly as it would have. Interactive shells
# also load ternarycode-integration.zsh, whose hooks install themselves at the first prompt.
#
# The user's .zshenv may define aliases, so everything that can be quoted is quoted.

if [[ -n "${TERNARYCODE_ZSH_ZDOTDIR+X}" ]]; then
    'builtin' 'export' ZDOTDIR="$TERNARYCODE_ZSH_ZDOTDIR"
    'builtin' 'unset' 'TERNARYCODE_ZSH_ZDOTDIR'
else
    'builtin' 'unset' 'ZDOTDIR'
fi

# try/always: the hooks are loaded even when the user's .zshenv fails or returns early.
{
    # zsh reads ${ZDOTDIR-$HOME}/.zshenv and skips unreadable files; so do we. Never source this
    # file again (a ZDOTDIR pointing back here).
    'builtin' 'typeset' _tc_file="${ZDOTDIR-$HOME}/.zshenv"
    if [[ -r "$_tc_file" && "${_tc_file:A}" != "${${(%):-%x}:A}" ]]; then
        'builtin' 'source' '--' "$_tc_file"
    fi
} always {
    if [[ -o 'interactive' ]]; then
        # ${(%):-%x} is this file; the hooks are next to it. They are parsed with aliases off.
        'builtin' 'typeset' _tc_file="${${(%):-%x}:A:h}/ternarycode-integration.zsh"
        [[ ! -r "$_tc_file" ]] || () {
            'builtin' 'emulate' -L zsh -o no_aliases
            'builtin' 'source' '--' "$1"
        } "$_tc_file"
    fi
    'builtin' 'unset' '_tc_file'
}
