# TernaryCode shell integration for fish: OSC 133 prompt marks (A prompt start, C command start,
# D;status command end) and the working directory (OSC 7).
#
# TernaryCode puts the directory above `fish/vendor_conf.d` first in XDG_DATA_DIRS, so fish loads
# this file on startup (before the user's config.fish). It takes that directory out again right
# away. fish 4.0 and later write OSC 133 and OSC 7 themselves (unless the `mark-prompt` feature
# is turned off); nothing is added then.

function __tc_restore_xdg_data_dirs
    set -q TERNARYCODE_FISH_XDG_DIR; or return
    if set -q XDG_DATA_DIRS
        set -l dirs (string split : -- "$XDG_DATA_DIRS")
        if set -l i (contains --index -- "$TERNARYCODE_FISH_XDG_DIR" $dirs)
            set -e dirs[$i]
        end
        # It was unset; fish's default list was put after ours.
        if set -q TERNARYCODE_FISH_XDG_UNSET; and test "$dirs" = "/usr/local/share /usr/share"
            set dirs
        end
        if set -q dirs[1]
            set -gx XDG_DATA_DIRS (string join : -- $dirs)
        else
            set -e -g XDG_DATA_DIRS
        end
    end
    set -e TERNARYCODE_FISH_XDG_DIR
    set -e TERNARYCODE_FISH_XDG_UNSET
end
__tc_restore_xdg_data_dirs
functions -e __tc_restore_xdg_data_dirs

function __tc_fish_marks_prompt
    set -l major (string match -r '^\d+' -- $version)
    test -n "$major"; and test "$major" -ge 4; or return 1
    # 0: on, 1: off, 2: unknown feature (4.0.0 to 4.0.5 always mark).
    status test-feature mark-prompt 2>/dev/null
    test $status -ne 1
end

if status --is-interactive; and not set -q __tc_loaded; and not __tc_fish_marks_prompt
    set -g __tc_loaded 1
    # 1 after C, until D.
    set -g __tc_state 0

    function __tc_prompt_start --on-event fish_prompt
        printf '\e]7;file://%s%s\a\e]133;A;redraw=1\a' $hostname (string escape --style=url -- $PWD)
    end

    function __tc_command_start --on-event fish_preexec
        set -g __tc_state 1
        printf '\e]133;C\a'
    end

    function __tc_command_end --on-event fish_postexec
        set -l s $status
        if test "$__tc_state" = 1
            printf '\e]133;D;%s\a' $s
        end
        set -g __tc_state 0
    end
end
functions -e __tc_fish_marks_prompt
