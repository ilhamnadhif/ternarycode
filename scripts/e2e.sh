#!/usr/bin/env bash
# End-to-end checks of the real app (debug build, macOS): each scenario runs TernaryCode with a
# private data folder and a `TERNARYCODE_DEV_SCRIPT`, injecting real keyboard and mouse events.
# Projects are temporary folders; nothing outside them is touched. A window appears while it runs.
#
#   scripts/e2e.sh            # all scenarios
#   scripts/e2e.sh editor     # only scenarios whose name contains "editor"
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
cargo build >/dev/null 2>&1 || { echo "build failed"; exit 1; }
# A language server answering from plain text: every scenario uses it (none starts a real one).
cargo build -p tc-lsp --bin tc-fake-lsp >/dev/null 2>&1 || { echo "fake language server build failed"; exit 1; }
FAKE_LSP="$ROOT/target/debug/tc-fake-lsp"
LSP="$FAKE_LSP"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/tc-e2e.XXXXXX")
BIN="$ROOT/target/debug/ternarycode"
ONLY="${1:-}"
PASS=0; FAIL=0

run() { # name, seconds, script
    local name="$1" secs="$2" script="$3"
    if [ -n "$ONLY" ] && [[ "$name" != *"$ONLY"* ]]; then return; fi
    local data="$WORK/data-$name"
    mkdir -p "$data"
    TERNARYCODE_DATA_DIR="$data" TERNARYCODE_SHOT_DIR="$WORK" TERNARYCODE_DEV_SCRIPT="$script" TERNARYCODE_DOWNLOAD_DIR="$WORK/downloads" \
        env ${LSP:+TERNARYCODE_LSP_OVERRIDE="$LSP"} \
        perl -e 'alarm shift; exec @ARGV' "$secs" "$BIN" > "$WORK/$name.log" 2>&1
    local p f
    p=$(grep -c "DEVTEST PASS" "$WORK/$name.log"); f=$(grep -c "DEVTEST FAIL" "$WORK/$name.log")
    local done; done=$(grep -c "DEVTEST DONE" "$WORK/$name.log")
    PASS=$((PASS + p)); FAIL=$((FAIL + f))
    if [ "$f" -eq 0 ] && { [ "$done" -gt 0 ] || [ "${4:-}" = "no-done" ]; }; then
        echo "ok    $name ($p checks)"
    else
        echo "FAIL  $name ($p passed, $f failed, done=$done)"; grep "DEVTEST FAIL" "$WORK/$name.log" | sed 's/^/        /'
        [ "$done" -eq 0 ] && [ "${4:-}" != "no-done" ] && FAIL=$((FAIL + 1))
    fi
}

# Fixtures: two plain folders and a git repository.
P1="$WORK/alpha"; P2="$WORK/beta"; R="$WORK/repo"
mkdir -p "$P1/src" "$P2" "$R"
printf 'fn main() {\n    println!("v1");\n}\n' > "$P1/src/main.rs"; printf 'notes v1\n' > "$P1/notes.txt"
(cd "$R" && git init -q -b main && printf 'keep\n' > keep.txt && printf 'line 1\nline 2\nline 3\n' > edit.txt \
    && printf 'bye\n' > gone.txt && git add -A && git -c user.name=t -c user.email=t@t commit -qm init)
# The agents' own folders (their earlier sessions) live here, not in the user's home: one Claude
# Code session of alpha.
export TERNARYCODE_AGENT_HOME="$WORK/agent-home"
CLAUDE_SESSIONS="$TERNARYCODE_AGENT_HOME/.claude/projects/$(cd "$P1" && pwd -P | sed 's/[^A-Za-z0-9]/-/g')"
mkdir -p "$CLAUDE_SESSIONS"
printf '%s\n' '{"type":"user","message":{"content":"fix the checkout"},"gitBranch":"main"}' '{"type":"ai-title","aiTitle":"Checkout bug"}' \
    > "$CLAUDE_SESSIONS/11111111-1111-1111-1111-111111111111.jsonl"
# A program the panes take for Claude Code by its name (it only sleeps), with no hooks.
mkdir -p "$WORK/bin"; ln -s /bin/sleep "$WORK/bin/claude"
# Links the app opens are written here instead of opening a browser.
OPENED="$WORK/opened.txt"; export TERNARYCODE_E2E_OPENED="$OPENED"; : > "$OPENED"
# A local server that says where it listens, as dev servers do, then listens there.
printf '#!/bin/sh\nprintf "  Local:   http://localhost:%%s/\\n" "$1"\nexec nc -lk "$1"\n' > "$WORK/bin/serve"; chmod +x "$WORK/bin/serve"
# A file to drop on a terminal (a screenshot, with a space in its name).
printf 'png' > "$WORK/drop me.png"

run smoke 40 "sleep 1200;; action add-project $P1;; sleep 1500;; keys echo hello-tc\\r;; wait-screen hello-tc;; wait-status idle;; expect-project alpha;; hook-latency claude PermissionRequest needs-input;; hook-latency claude Stop idle|done;; quit"

# ⌘J jumps to the waiting agent and back; ⌘1 is beta, listed first while it is under Active.
run jump 40 "sleep 1000;; action add-project $P2;; sleep 1000;; action add-project $P1;; sleep 1200;; hook 2 claude Notification permission_prompt;; sleep 400;; wait-queue 1;; action next-attention;; sleep 400;; expect-project beta;; wait-queue 1;; key enter;; sleep 400;; wait-queue 0;; action back-attention;; sleep 400;; expect-project alpha;; action select 0;; sleep 400;; expect-project beta;; quit"

# A question asked in the pane being looked at still waits for the user until it is answered.
run focused-question 40 "sleep 1000;; action add-project $P1;; sleep 1200;; hook 2 claude PermissionRequest;; sleep 400;; wait-queue 1;; expect-shown attention-summary;; key enter;; sleep 400;; wait-queue 0;; quit"

# Claude Code typed by hand with no hooks installed: after a few seconds the status bar (and a
# Grid tile say it is not connected.
run unconnected 40 "sleep 1000;; action add-project $P1;; sleep 1200;; keys $WORK/bin/claude 60\\r;; sleep 7000;; expect-shown unconnected;; shot e2e-unconnected;; key ctrl+c;; sleep 800;; expect-hidden unconnected;; quit"

# Run Agent lists the project's earlier Claude Code sessions.
# Run Agent in a project just opened: the agent starts in its unused shell (no second tab); run
# again, the shell is in use, so it gets a pane of its own. A preset that only echoes.
mkdir -p "$WORK/data-run-here"; printf '{"presets":[{"id":"echoer","name":"Echoer","command":"echo agent-ran-here"}]}\n' > "$WORK/data-run-here/settings.json"
run run-here 40 "sleep 1000;; action add-project $P1;; sleep 1200;; wait-tabs 0:2;; action run echoer;; wait-screen agent-ran-here;; wait-tabs 0:2;; action run echoer;; sleep 800;; wait-tabs 0:2,3;; quit"
# A shell whose startup files ask something first (oh-my-zsh's "Would you like to update? [Y/n]",
# which reads one key): Run Agent waits for the shell's prompt, so the question does not take the
# first letter of the command (`claude` once ran as `laude`); it is typed once answered.
mkdir -p "$WORK/zdot"; printf 'printf "[oh-my-zsh] Would you like to update? [Y/n] "\nread -r -k 1 option\necho\n[[ "$option" == [yY] ]] || echo "[oh-my-zsh] You can update manually by running omz update"\n' > "$WORK/zdot/.zshrc"
mkdir -p "$WORK/data-run-asks"; printf '{"presets":[{"id":"echoer","name":"Echoer","command":"echo agent-ran-$((40+2))"}]}\n' > "$WORK/data-run-asks/settings.json"
export ZDOTDIR="$WORK/zdot"
run run-asks 40 "sleep 1000;; action add-project $P1;; sleep 300;; action run echoer;; wait-screen Would you like to update;; sleep 1500;; expect-screen-count command not found 0;; expect-screen-count agent-ran-42 0;; keys n;; wait-screen agent-ran-42;; expect-screen-count command not found 0;; quit"
unset ZDOTDIR
# Run Agent when the agent's program is not on the PATH: a dialog says how to install it and
# nothing is typed; Run Anyway types the command all the same (an alias the check can't see).
mkdir -p "$WORK/data-agent-missing"; printf '{"presets":[{"id":"codex","name":"Codex","agent":"codex","command":"tc-no-such-agent"}]}\n' > "$WORK/data-agent-missing/settings.json"
run agent-missing 40 "sleep 1000;; action add-project $P1;; sleep 3000;; action run-agent codex;; sleep 600;; expect-dialog open;; expect-shown dialog-anyway;; expect-screen-count tc-no-such-agent 0;; click dialog-anyway;; sleep 500;; expect-dialog closed;; wait-screen tc-no-such-agent;; quit"
# Run Agent's first choice in a shell used before: at its prompt with nothing typed, the agent
# starts in it ("This Tab"); with something typed on its command line, in a new tab.
mkdir -p "$WORK/data-run-here-idle"; cp "$WORK/data-run-here/settings.json" "$WORK/data-run-here-idle/settings.json"
run run-here-idle 40 "sleep 1000;; action add-project $P1;; sleep 1200;; wait-tabs 0:2;; keys echo used-before\\r;; wait-screen used-before;; sleep 800;; action run-agent echoer;; wait-screen agent-ran-here;; wait-tabs 0:2;; sleep 800;; keys ls;; sleep 400;; action run-agent echoer;; sleep 800;; wait-tabs 0:2 1:3;; quit"
# A shell an earlier run started (handed over in an update) lacks the variables this run's agent
# flags name: Run Agent starts the agent in a new tab, not there.
mkdir -p "$WORK/data-run-here-adopted"; cp "$WORK/data-run-here/settings.json" "$WORK/data-run-here-adopted/settings.json"
run run-here-adopted 50 "sleep 1000;; action add-project $P1;; sleep 1500;; keys echo before-update\\r;; wait-screen before-update;; restart-in-place;; sleep 2500;; wait-screen before-update;; wait-tabs 0:2;; sleep 800;; action run-agent echoer;; sleep 800;; wait-tabs 0:2 1:3;; wait-screen agent-ran-here;; quit"
# An update restarts the app in place: the panes' programs keep running in the same shell. The
# status and an agent's hook token carry over; a program reading its input (cat) still gets the
# keys; the screen is as it was. Twice, so a run taken over can hand over again.
run restart-in-place 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys echo marker-one\\r;; wait-screen marker-one;; hook 2 claude PermissionRequest;; wait-status needs-input;; restart-in-place;; sleep 2500;; wait-screen marker-one;; wait-status needs-input;; hook 2 claude Stop;; wait-status idle|done;; keys echo \$\$ > $WORK/pid-before; cat\\r;; sleep 500;; keys typed-before\\r;; wait-screen typed-before;; restart-in-place;; sleep 2500;; wait-screen typed-before;; keys typed-after\\r;; sleep 400;; key ctrl+d;; sleep 400;; keys [ \"\$(cat $WORK/pid-before)\" = \"\$\$\" ] && echo same-shell-\$((40+2))\\r;; wait-screen same-shell-42;; quit"
run run-agent-sessions 40 "sleep 1000;; action add-project $P1;; sleep 1200;; action pick;; sleep 1000;; expect-dialog open;; expect-shown session:0;; expect-shown session:1;; expect-hidden session:2;; key down;; sleep 300;; shot e2e-run-agent-sessions;; key esc;; sleep 300;; expect-dialog closed;; quit"

# The sidebar's project search: it narrows the list, Enter opens the chosen project and empties
# it, ↓ moves the choice, Esc empties it.
run project-search 40 "sleep 1000;; action add-project $P1;; sleep 1000;; action add-project $P2;; sleep 1200;; expect-project beta;; click project-filter;; sleep 300;; keys alp;; sleep 400;; expect-shown project:alpha;; expect-hidden project:beta;; shot e2e-project-search;; key enter;; sleep 400;; expect-project alpha;; expect-shown project:beta;; click project-filter;; sleep 300;; key down;; key enter;; sleep 400;; expect-project beta;; click project-filter;; sleep 300;; keys zzz;; sleep 400;; expect-hidden project:alpha;; expect-hidden project:beta;; key esc;; sleep 400;; expect-shown project:alpha;; expect-shown project:beta;; quit"

# ⌘-click on a web address opens it, without the brackets and the full stop around it, also in
# a program that takes the mouse (mouse reporting on): the click is not reported to it. (Web
# pages open in the default browser here: Settings → Browser's switch off; `browser` below has
# them open in TernaryCode.)
for name in term-link local-server; do mkdir -p "$WORK/data-$name"; printf '{"browser_here":false}\n' > "$WORK/data-$name/settings.json"; done
URL_A="https://example.com/opened-from-the-terminal-by-cmd-click"
URL_B="https://example.com/opened-while-the-program-takes-the-mouse"
run term-link 40 "sleep 1000;; action add-project $P1;; sleep 1200;; keys clear; for i in 1 2 3 4 5 6; do echo '($URL_A).'; done\\r;; wait-screen opened-from;; sleep 300;; mouse cmd-move pane:2@0.2,0.06;; sleep 300;; mouse cmd-down pane:2@0.2,0.06;; mouse cmd-up pane:2@0.2,0.06;; sleep 500;; expect-file $OPENED $URL_A\\n;; keys clear; for i in 1 2 3 4 5 6; do echo '($URL_B).'; done; printf '\\033[?1000h'; sleep 30\\r;; wait-screen opened-while;; sleep 300;; mouse cmd-move pane:2@0.2,0.06;; sleep 300;; mouse cmd-down pane:2@0.2,0.06;; mouse cmd-up pane:2@0.2,0.06;; sleep 500;; expect-file $OPENED $URL_B\\n;; key ctrl+c;; keys printf '\\033[?1000l'\\r;; quit"

# A file dropped on a terminal: its path is typed there, escaped (an agent attaches a screenshot
# that way), and no project is added; a folder dropped on the sidebar becomes a project.
run drop-path 40 "sleep 1000;; action add-project $P1;; sleep 1200;; drop pane:2 $WORK/drop me.png;; wait-screen drop\\ me.png;; expect-project alpha;; drop project-filter $P2;; sleep 1200;; expect-shown project:beta;; quit"

# A dev server that says where it listens: its address shows in the project header while it
# answers, opens on a click, and goes when the server stops.
run local-server 40 "sleep 1000;; action add-project $P1;; sleep 1200;; keys $WORK/bin/serve 47613\\r;; wait-shown server-chip;; shot e2e-local-server;; click server-chip;; sleep 400;; expect-file $OPENED http://localhost:47613/;; key ctrl+c;; wait-hidden server-chip;; quit"

# Web pages in TernaryCode (macOS): a dev server announced in the terminal (the page served by
# a local web server); its chip opens the page in the project's Browser panel beside the
# terminal, where the web view sits exactly on the page's place; an address typed goes there;
# a new page (⌥⇧⌘B) opens the project's server beside it in the panel; a page tab brings its
# page back. After a restart both pages come back where they were.
if [ "$(uname)" = Darwin ] && { [ -z "$ONLY" ] || [[ "browser-1 browser-2 browser-agent browser-agent-popup browser-dialogs browser-agent-dialogs browser-agent-human browser-typing browser-isolation browser-agent-show browser-update browser-tools browser-more browser-focus browser-fullscreen browser-popup browser-grid browser-button browser-panel browser-migrate opencode-env split-resize" == *"$ONLY"* ]]; }; then
    SITE="$WORK/site"; mkdir -p "$SITE"
    printf '<!doctype html><title>TC Test Page</title><h1>Hello from the dev server</h1><a href="/second.html">Second page</a>\n' > "$SITE/index.html"
    printf '<!doctype html><title>Second</title><h2>Second page</h2>\n' > "$SITE/second.html"
    WEBPORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    python3 -m http.server "$WEBPORT" --bind 127.0.0.1 --directory "$SITE" >/dev/null 2>&1 & WEB_PID=$!; disown
    printf '#!/bin/sh\nprintf "  Local:   http://localhost:%%s/\\n" "$1"\nexec sleep 600\n' > "$WORK/bin/announce"; chmod +x "$WORK/bin/announce"
    run browser-1 60 "sleep 1000;; action add-project $P1;; sleep 1200;; keys $WORK/bin/announce $WEBPORT\\r;; wait-shown server-chip;; click server-chip;; wait-browser TC Test Page;; wait-tabs 0:2;; wait-pages 3*;; wait-panel shown;; sleep 500;; expect-web-placed;; web-shot e2e-browser-page;; shot e2e-browser;; click browser-address:3;; sleep 200;; key cmd+a;; keys localhost:$WEBPORT/second.html\\r;; wait-browser Second;; action new-browser-tab;; wait-tabs 0:2;; wait-pages 3,4*;; wait-browser TC Test Page;; sleep 500;; expect-web-placed;; action palette;; sleep 500;; expect-web-hidden;; key esc;; sleep 500;; expect-web-placed;; click browser-more:4;; sleep 400;; expect-web-hidden;; shot e2e-browser-menu;; key esc;; sleep 500;; expect-web-placed;; click page-tab:3;; wait-pages 3*,4;; wait-browser Second;; sleep 500;; expect-web-placed;; quit"
    mkdir -p "$WORK/data-browser-2"; cp "$WORK/data-browser-1/workspace.json" "$WORK/data-browser-2/" 2>/dev/null
    run browser-2 50 "sleep 2000;; expect-project alpha;; wait-tabs 0:2;; wait-pages 3*,4;; wait-panel shown;; wait-browser Second;; sleep 500;; expect-web-placed;; click page-tab:4;; wait-browser TC Test Page;; sleep 500;; expect-web-placed;; quit"

    # An agent using the browser through its tools: the MCP server (`ternarycode-hook mcp`) runs
    # in the pane as an agent starts it, with each tool call piped in (`m NAME` sends
    # NAME.json and writes NAME.out). The first call opens the page beside the pane; the agent
    # clicks, types, reads the console, runs a script, takes a picture, follows a link and
    # comes back. A link to another site asks first (Block); an address elsewhere asked and
    # allowed for the project opens without asking the next time. Take Over answers the call
    # waiting, and the next one, "the user has the browser"; Hand Back lets them through.
    printf '<!doctype html><title>Agent Page</title><h1>Shop</h1><p id="count">Count: 0</p><script>let n = 0; function add() { n++; document.getElementById("count").textContent = "Count: " + n; console.log("clicked", n); }</script><button onclick="add()">Add one</button><input placeholder="Search"><a href="/second.html">Second page</a> <a href="https://example.com/">Elsewhere</a>\n' > "$SITE/agent.html"
    MCP="$WORK/mcp"; mkdir -p "$MCP"
    call() { printf '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"%s","arguments":%s}}\n' "$2" "$3" > "$MCP/$1.json"; }
    call nav browser_navigate "{\"url\":\"localhost:$WEBPORT/agent.html\"}"
    call add browser_click '{"ref":"e1","element":"Add one button"}'
    call typ browser_type '{"ref":"e2","element":"Search field","text":"linen"}'
    call con browser_console_messages '{}'
    call js browser_evaluate '{"function":"() => document.title + \" / \" + n"}'
    call pic browser_take_screenshot '{}'
    call sec browser_click '{"ref":"e3","element":"Second page link"}'
    call bak browser_navigate_back '{}'
    call away browser_click '{"ref":"e4","element":"Elsewhere link"}'
    call inv browser_navigate '{"url":"http://tc-e2e.invalid/"}'
    call inv2 browser_navigate '{"url":"http://tc-e2e.invalid/two"}'
    call hold browser_wait_for '{"text":"never shows","time":20}'
    call snap browser_snapshot '{}'
    call scr browser_scroll '{"direction":"down"}'
    mkdir -p "$WORK/data-browser-agent"; printf '{"browser_agent_js":true,"browser_agent_ask":true}\n' > "$WORK/data-browser-agent/settings.json"
    HOOK="$ROOT/target/debug/ternarycode-hook"
    run browser-agent 120 "sleep 1000;; action add-project $P1;; sleep 1500;; keys m() { $HOOK mcp claude < $MCP/\$1.json > $MCP/\$1.out; echo ok-\$1; }; clear\\r;; sleep 300;; keys m nav\\r;; wait-screen ok-nav;; wait-tabs 0:2;; wait-pages 3*;; wait-panel shown;; expect-file $MCP/nav.out Page: Agent Page;; expect-file $MCP/nav.out [ref=e1];; wait-browser Agent Page;; keys m add\\r;; wait-screen ok-add;; expect-file $MCP/add.out Clicked Add one button.;; expect-file $MCP/add.out Count: 1;; keys m typ\\r;; wait-screen ok-typ;; expect-file $MCP/typ.out linen;; keys m con\\r;; wait-screen ok-con;; expect-file $MCP/con.out [log] clicked 1;; keys m js\\r;; wait-screen ok-js;; expect-file $MCP/js.out Agent Page / 1;; keys m pic\\r;; wait-screen ok-pic;; expect-file $MCP/pic.out image/png;; keys m sec\\r;; wait-screen ok-sec;; expect-file $MCP/sec.out Page: Second;; keys m bak\\r;; wait-screen ok-bak;; expect-file $MCP/bak.out Page: Agent Page;; keys m scr\\r;; wait-screen ok-scr;; expect-file $MCP/scr.out Scrolled down.;; hook 2 claude SessionStart;; keys m away &\\r;; wait-shown browser-ask:3;; wait-status needs-input;; expect-web-placed;; shot e2e-browser-ask;; click browser-block:3;; wait-screen ok-away;; wait-status running;; expect-file $MCP/away.out Blocked by the user: example.com;; wait-browser Agent Page;; keys m inv &\\r;; wait-shown browser-ask:3;; click browser-always:3;; wait-screen ok-inv;; expect-file $MCP/inv.out Opened http://tc-e2e.invalid/;; keys m inv2\\r;; wait-screen ok-inv2;; expect-file $MCP/inv2.out Opened http://tc-e2e.invalid/two;; expect-hidden browser-ask:3;; keys clear; m nav\\r;; wait-screen ok-nav;; keys m hold &\\r;; sleep 1000;; wait-shown browser-take-over:3;; shot e2e-browser-agent;; click browser-take-over:3;; sleep 400;; expect-shown browser-take-over:3;; focus-pane 0;; wait-screen ok-hold;; expect-file $MCP/hold.out took the browser over from Claude Code;; keys m snap\\r;; wait-screen ok-snap;; expect-file $MCP/snap.out has the browser now;; click browser-take-over:3;; sleep 300;; keys clear; m snap\\r;; wait-screen ok-snap;; expect-file $MCP/snap.out Page: Agent Page;; quit"
    expect_saved() { grep -q "$2" "$WORK/data-$1/workspace.json" && echo "ok    $1 saved $2" || { echo "FAIL  $1 did not save $2"; FAIL=$((FAIL + 1)); }; }
    [ -z "$ONLY" ] || [[ "browser-agent" == *"$ONLY"* ]] && expect_saved browser-agent tc-e2e.invalid

    # An agent's click that opens a sign-in pop-up: the pop-up opens tied to the page, the
    # agent hears of it and its next steps act in it; its click there signs the page in and the
    # pop-up closes, after which the steps act in the page again; browser_navigate closes an open
    # pop-up and goes on in the page. A pop-up to a site the agent may not open is asked about
    # first (Block: not opened).
    printf '<!doctype html><title>Agent Popup Opener</title><button onclick="window.open(&quot;/agentsignin.html&quot;, &quot;s&quot;, &quot;width=480,height=560&quot;)">Sign in</button><button onclick="window.open(&quot;http://tc-popup.invalid/x&quot;, &quot;w&quot;, &quot;width=400,height=400&quot;)">Sign in elsewhere</button><script>window.addEventListener("message", function (e) { if (e.origin === location.origin) document.title = "Signed in: " + e.data; });</script>\n' > "$SITE/agentpop.html"
    printf '<!doctype html><title>Agent Sign In</title><button onclick="window.opener.postMessage(&quot;agent-ok&quot;, &quot;*&quot;); window.close();">Continue</button>\n' > "$SITE/agentsignin.html"
    call pnav browser_navigate "{\"url\":\"localhost:$WEBPORT/agentpop.html\"}"
    call pclick browser_click '{"ref":"e1","element":"Sign in button"}'
    call pcont browser_click '{"ref":"e1","element":"Continue button"}'
    call pclick2 browser_click '{"ref":"e1","element":"Sign in button"}'
    call pnav2 browser_navigate "{\"url\":\"localhost:$WEBPORT/agentpop.html\"}"
    call pelse browser_click '{"ref":"e2","element":"Sign in elsewhere button"}'
    # The calls run from a preset (no typing: the keyboard is not needed while the user works).
    printf 'm() { "%s" mcp claude < "%s/$1.json" > "%s/$1.out"; echo "ok-$1"; }\nm pnav; m pclick; sleep 3; m pcont; m pclick2; sleep 2; m pnav2; m pelse\n' "$HOOK" "$MCP" "$MCP" > "$WORK/agentpop.sh"
    mkdir -p "$WORK/data-browser-agent-popup"; printf '{"browser_agent_ask":true,"presets":[{"id":"popper","name":"Popper","command":"sh %s"}]}\n' "$WORK/agentpop.sh" > "$WORK/data-browser-agent-popup/settings.json"
    run browser-agent-popup 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action run popper;; wait-screen ok-pnav;; expect-file $MCP/pnav.out Page: Agent Popup Opener;; wait-screen ok-pclick;; wait-popups 1;; expect-file $MCP/pclick.out A pop-up window opened;; expect-file $MCP/pclick.out Page: Agent Sign In (a pop-up window);; wait-screen ok-pcont;; wait-popups 0;; expect-file $MCP/pcont.out The pop-up window closed;; expect-file $MCP/pcont.out Page: Signed in: agent-ok;; wait-pages 3*;; wait-screen ok-pclick2;; wait-popups 1;; wait-screen ok-pnav2;; wait-popups 0;; expect-file $MCP/pnav2.out The pop-up window closed;; expect-file $MCP/pnav2.out Page: Agent Popup Opener;; wait-shown browser-ask:3;; wait-popups 0;; click browser-block:3;; wait-screen ok-pelse;; expect-file $MCP/pelse.out Blocked by the user: tc-popup.invalid;; wait-popups 0;; quit"
    # A page's dialogs show as a bar above it and the page goes on with the answer (Cancel, OK, a
    # prompt's text); a download lands in the downloads folder and is listed on the page. An
    # agent is told of a dialog its click opened, answers it with browser_handle_dialog, and hears
    # where a download went.
    DL="$WORK/downloads"; mkdir -p "$DL"; printf 'hello download\n' > "$SITE/file.txt"
    printf '<!doctype html><title>Dialogs</title><button id="rm" onclick="document.title = confirm(&quot;Remove Wool Beanie?&quot;) ? &quot;Removed&quot; : &quot;Kept&quot;">Remove</button><button id="hi" onclick="alert(&quot;Hello there&quot;); document.title = &quot;Alerted&quot;">Hello</button><button id="nm" onclick="document.title = &quot;Name: &quot; + prompt(&quot;Your name?&quot;, &quot;Dina&quot;)">Name</button><a id="dl" href="/file.txt" download>Get the file</a>\n' > "$SITE/dialogs.html"
    run browser-dialogs 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web http://localhost:$WEBPORT/dialogs.html;; wait-browser Dialogs;; wait-pages 3*;; sleep 800;; web-click document.getElementById('rm');; wait-dialog Remove Wool Beanie;; expect-shown browser-dialog:3;; shot e2e-browser-dialog;; click browser-dialog-cancel:3;; wait-browser Kept;; expect-hidden browser-dialog:3;; web-click document.getElementById('rm');; wait-dialog Remove Wool Beanie;; click browser-dialog-ok:3;; wait-browser Removed;; web-click document.getElementById('hi');; wait-dialog Hello there;; click browser-dialog-ok:3;; wait-browser Alerted;; web-click document.getElementById('nm');; wait-dialog Your name;; click browser-dialog-ok:3;; wait-browser Name: Dina;; web-click document.getElementById('dl');; wait-download file.txt;; expect-file $DL/file.txt hello download;; expect-shown browser-downloads:3;; quit"
    call dnav browser_navigate "{\"url\":\"localhost:$WEBPORT/dialogs.html\"}"
    call drm browser_click '{"ref":"e1","element":"Remove button"}'
    call dno browser_handle_dialog '{"accept":false}'
    call dnm browser_click '{"ref":"e3","element":"Name button"}'
    call dyes browser_handle_dialog '{"accept":true,"promptText":"Ada"}'
    call ddl browser_click '{"ref":"e4","element":"Get the file link"}'
    printf 'm() { "%s" mcp claude < "%s/$1.json" > "%s/$1.out"; echo "ok-$1"; }\nm dnav; m drm; m dno; m dnm; m dyes; m ddl\n' "$HOOK" "$MCP" "$MCP" > "$WORK/agentdlg.sh"
    mkdir -p "$WORK/data-browser-agent-dialogs"; printf '{"presets":[{"id":"dlg","name":"Dialogs","command":"sh %s"}]}\n' "$WORK/agentdlg.sh" > "$WORK/data-browser-agent-dialogs/settings.json"
    # The file the scenario before downloaded goes (a download onto it would fail).
    rm -f "$DL/file.txt"
    run browser-agent-dialogs 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action run dlg;; wait-screen ok-ddl;; expect-file $MCP/drm.out A dialog is open in the page (confirm;; expect-file $MCP/drm.out Remove Wool Beanie?;; expect-file $MCP/dno.out Dismissed the dialog;; expect-file $MCP/dno.out Page: Kept;; expect-file $MCP/dnm.out A dialog is open in the page (prompt;; expect-file $MCP/dyes.out Page: Name: Ada;; expect-file $MCP/ddl.out Downloaded file.txt to $DL/file.txt;; quit"
    # A page's tools: the find bar counts and steps through matches (round), the zoom steps
    # (WebKit draws the page at it; the toolbar's chip shows it and a click resets it), and the
    # Web Inspector opens.
    printf '<!doctype html><title>Find Page</title><p>Linen shirt</p><p>Wool beanie</p><p>linen tote and LINEN scarf</p>\n' > "$SITE/find.html"
    run browser-tools 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web http://localhost:$WEBPORT/find.html;; wait-browser Find Page;; wait-pages 3*;; sleep 800;; page-find linen;; wait-page-find 1/3;; expect-shown browser-find:3;; page-find-next;; wait-page-find 2/3;; page-find-next;; wait-page-find 3/3;; page-find-next;; wait-page-find 1/3;; page-find-prev;; wait-page-find 3/3;; shot e2e-browser-find;; page-find zzz;; wait-page-find 0/0;; page-zoom 1;; sleep 400;; expect-page-zoom 1.1;; expect-shown browser-zoom:3;; page-zoom 1;; sleep 400;; expect-page-zoom 1.25;; click browser-zoom:3;; sleep 400;; expect-page-zoom 1;; expect-hidden browser-zoom:3;; page-inspect;; wait-inspector;; page-print;; sleep 1500;; shot e2e-browser-print;; key esc;; sleep 800;; expect-web-placed;; quit"
    # An agent acting as a person: real keys type and submit, the pointer hovers, drags (pointer
    # events, and the page's own drag and drop), a file field gets a project file (one outside the
    # project and Downloads is refused), and a frame of another site (127.0.0.1 in a localhost
    # page) takes clicks and keys at points of the page.
    printf 'upload me\n' > "$P1/upload.txt"
    printf '<!doctype html><title>Pay Frame</title><body style="margin:0"><input id="card" style="position:absolute;left:10px;top:10px;width:200px;height:30px"><button style="position:absolute;left:10px;top:60px;width:100px;height:30px" onclick="parent.postMessage(&quot;paid:&quot; + document.getElementById(&quot;card&quot;).value, &quot;*&quot;)">Pay</button></body>\n' > "$SITE/frame.html"
    printf '<!doctype html><title>Human Page</title><body style="margin:0;font:14px sans-serif"><form onsubmit="event.preventDefault(); document.title = &quot;Submitted: &quot; + this.q.value"><input name="q" aria-label="Search field" style="position:absolute;left:10px;top:10px;width:150px"><button style="position:absolute;left:170px;top:10px">Search</button></form><div role="button" style="position:absolute;left:10px;top:50px;width:100px;height:30px;background:#ddd" onmouseenter="document.title = &quot;Hovered&quot;">Hover me</div><div id="a" role="button" style="position:absolute;left:10px;top:90px;width:80px;height:30px;background:#cde">Drag A</div><div id="b" role="button" style="position:absolute;left:200px;top:90px;width:80px;height:30px;background:#edc">Target B</div><div role="button" draggable="true" style="position:absolute;left:10px;top:130px;width:80px;height:30px;background:#dec" ondragstart="event.dataTransfer.setData(&quot;text&quot;, &quot;c&quot;)">Card C</div><div role="button" style="position:absolute;left:200px;top:130px;width:80px;height:30px;background:#ced" ondragover="event.preventDefault()" ondrop="document.title = &quot;HTML5 drop &quot; + event.dataTransfer.getData(&quot;text&quot;)">Drop D</div><input type="file" aria-label="Receipt" style="position:absolute;left:10px;top:170px" onchange="document.title = &quot;File: &quot; + this.files[0].name"><iframe title="Pay" src="http://127.0.0.1:%s/frame.html" style="position:absolute;left:20px;top:220px;width:320px;height:120px;border:0"></iframe><script>let down = false; document.getElementById("a").addEventListener("pointerdown", () => down = true); document.addEventListener("pointerup", (e) => { const t = document.elementFromPoint(e.clientX, e.clientY); if (down && t && t.id === "b") document.title = "Dropped on B"; down = false; }); addEventListener("message", (e) => { if (String(e.data).startsWith("paid:")) document.title = "Paid: " + String(e.data).slice(5); });</script></body>\n' "$WEBPORT" > "$SITE/human.html"
    call hnav browser_navigate "{\"url\":\"localhost:$WEBPORT/human.html\"}"
    call hq browser_click '{"ref":"e1","element":"Search field"}'
    call htype browser_keyboard_type '{"text":"hello","submit":true}'
    call hhover browser_hover '{"ref":"e3","element":"Hover me"}'
    call hdrag browser_drag '{"startRef":"e4","startElement":"Drag A","endRef":"e5","endElement":"Target B"}'
    call hdnd browser_drag '{"startRef":"e6","startElement":"Card C","endRef":"e7","endElement":"Drop D"}'
    call hup browser_file_upload '{"ref":"e8","element":"Receipt","paths":["upload.txt"]}'
    call hbad browser_file_upload '{"ref":"e8","element":"Receipt","paths":["/etc/hosts"]}'
    call hshot browser_take_screenshot '{}'
    call hxy1 browser_mouse_click_xy '{"x":130,"y":245,"element":"Card number in the Pay frame"}'
    call htype2 browser_keyboard_type '{"text":"4242"}'
    call hxy2 browser_mouse_click_xy '{"x":80,"y":295,"element":"Pay in the Pay frame"}'
    printf 'm() { "%s" mcp claude < "%s/$1.json" > "%s/$1.out"; echo "ok-$1"; }\nm hnav; m hq; m htype; m hhover; m hdrag; m hdnd; m hup; m hbad; m hshot; m hxy1; m htype2; m hxy2\n' "$HOOK" "$MCP" "$MCP" > "$WORK/agenthuman.sh"
    mkdir -p "$WORK/data-browser-agent-human"; printf '{"presets":[{"id":"hum","name":"Human","command":"sh %s"}]}\n' "$WORK/agenthuman.sh" > "$WORK/data-browser-agent-human/settings.json"
    run browser-agent-human 90 "sleep 1000;; action add-project $P1;; sleep 1500;; action run hum;; wait-screen ok-hdrag;; wait-screen ok-hbad;; wait-screen ok-hxy1;; wait-screen ok-hxy2;; expect-file $MCP/hnav.out [ref=e8];; expect-file $MCP/htype.out Page: Submitted: hello;; expect-file $MCP/hhover.out Page: Hovered;; expect-file $MCP/hdrag.out Page: Dropped on B;; expect-file $MCP/hdnd.out Page: HTML5 drop c;; expect-file $MCP/hup.out Page: File: upload.txt;; expect-file $MCP/hbad.out outside the project;; expect-file $MCP/hshot.out CSS pixels;; expect-file $MCP/hxy2.out Page: Paid: 4242;; window-shot e2e-agent-page;; quit"
    # An agent typing (macOS): keys the page does not take (nothing in it has the keyboard) go
    # nowhere, never into the terminal; text typed into an editor stays in order, though the
    # page got the keyboard back and forth between calls; the user's keys meanwhile go to the
    # terminal, not into the agent's text. Text typed after a click made in an earlier call goes
    # where the click put the caret, even when the user clicks the terminal while it is typed.
    printf '<!doctype html><title>Typing Page</title><body style="margin:0;font:14px sans-serif"><div contenteditable="true" aria-label="Note" style="position:absolute;left:10px;top:10px;width:400px;min-height:40px;border:1px solid #888"></div><div contenteditable="true" aria-label="Greeting" style="position:absolute;left:10px;top:120px;width:600px;border:1px solid #888">Hello world</div><p style="position:absolute;left:10px;top:300px">Nothing here</p>\n' > "$SITE/typing.html"
    call tnav browser_navigate "{\"url\":\"localhost:$WEBPORT/typing.html\"}"
    call tblank browser_mouse_click_xy '{"x":300,"y":200,"element":"empty space"}'
    call tleak browser_keyboard_type "{\"text\":\"echo leaked > $MCP/leak.txt\\n\"}"
    call tenter browser_press_key '{"key":"Enter"}'
    call tnote browser_click '{"ref":"e1","element":"Note"}'
    call tkt browser_keyboard_type '{"text":"The quick brown fox jumps over the lazy dog, then the quick brown fox naps.\nLine two."}'
    call tget browser_evaluate '{"function":"() => document.querySelector(\"[contenteditable]\").innerText"}'
    call tgreet browser_click '{"ref":"e2","element":"Greeting"}'
    call tgkt browser_keyboard_type '{"text":", typed in the next call while the user clicks the terminal."}'
    call tgget browser_evaluate '{"function":"() => document.querySelector(\"[aria-label=Greeting]\").innerText"}'
    mkdir -p "$WORK/data-browser-typing"; printf '{"browser_agent_js":true}\n' > "$WORK/data-browser-typing/settings.json"
    run browser-typing 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys m() { $HOOK mcp claude < $MCP/\$1.json > $MCP/\$1.out; echo ok-\$1; }; clear\\r;; sleep 300;; keys m tnav\\r;; wait-screen ok-tnav;; wait-browser Typing Page;; keys m tblank\\r;; wait-screen ok-tblank;; keys m tleak\\r;; wait-screen ok-tleak;; keys m tenter\\r;; wait-screen ok-tenter;; sleep 800;; expect-not-file $MCP/leak.txt leaked;; keys m tnote\\r;; wait-screen ok-tnote;; keys m tkt\\r;; sleep 400;; keys qq;; wait-screen qqok-tkt;; clear-line;; keys m tget\\r;; wait-screen ok-tget;; expect-file $MCP/tget.out The quick brown fox jumps over the lazy dog, then the quick brown fox naps.;; expect-file $MCP/tget.out Line two.;; expect-not-file $MCP/tget.out qq;; keys m tgreet\\r;; wait-screen ok-tgreet;; keys m tgkt\\r;; sleep 400;; click pane:2;; wait-screen ok-tgkt;; keys m tgget\\r;; wait-screen ok-tgget;; expect-file $MCP/tgget.out Hello world, typed in the next call while the user clicks the terminal.;; quit"
    # The user's keyboard stays where the user types, whatever pages and agents do (macOS):
    # moved to the terminal by a key (not a click), it leaves the page; an agent types in its
    # page while the user types in the terminal (each key to its own place, ⌘J to the app's
    # menu) and gives the keyboard back after; a click in a page of the user's and keys typed at once go there; a
    # click in the terminal takes the keyboard from that page; the agent's keys wait while the
    # user types in another page, and neither text loses a key; a page that keeps focusing a
    # field of its own does not take the user's keys.
    printf '<!doctype html><title>Iso Page</title><body style="margin:0;font:14px sans-serif"><textarea id="t" aria-label="Notes" style="position:absolute;left:10px;top:10px;width:500px;height:80px"></textarea></body>\n' > "$SITE/iso.html"
    printf '<!doctype html><title>User Page</title><body style="margin:0;font:14px sans-serif"><input id="u" aria-label="User field" style="position:absolute;left:10px;top:10px;width:300px"></body>\n' > "$SITE/user.html"
    printf '<!doctype html><title>Self Page</title><body style="margin:0;font:14px sans-serif"><input id="s" aria-label="Self field" style="position:absolute;left:10px;top:10px;width:300px"><script>setInterval(() => document.getElementById("s").focus(), 150);</script></body>\n' > "$SITE/self.html"
    call inav browser_navigate "{\"url\":\"localhost:$WEBPORT/iso.html\"}"
    call inote browser_click '{"ref":"e1","element":"Notes"}'
    call ikt browser_keyboard_type '{"text":"Typed by the agent, key by key, while the user types in the terminal."}'
    call ikt2 browser_keyboard_type '{"text":" And more."}'
    call iget browser_evaluate '{"function":"() => document.querySelector(\"#t\").value"}'
    call iclk browser_click '{"ref":"e1"}'
    mkdir -p "$WORK/data-browser-isolation"; printf '{"browser_agent_js":true}\n' > "$WORK/data-browser-isolation/settings.json"
    run browser-isolation 120 "sleep 1000;; action add-project $P1;; sleep 1500;; keys m() { $HOOK mcp claude < $MCP/\$1.json > $MCP/\$1.out; echo ok-\$1; }; clear; echo ready-\$((1+1))\\r;; wait-screen ready-2;; keys m inav\\r;; wait-screen ok-inav;; wait-browser Iso Page;; sleep 500;; page-focus;; wait-page-focus;; sleep 300;; expect-page-keyboard yes;; focus-pane 0;; sleep 600;; expect-page-focus no;; expect-page-keyboard no;; keys m inote\\r;; wait-screen ok-inote;; sleep 300;; keys m ikt\\r;; sleep 700;; keys zz;; sleep 200;; key cmd+j;; sleep 200;; keys yy;; window-shot e2e-agent-typing;; wait-screen zzyyok-ikt;; wait-toast No agent is waiting for you;; sleep 600;; expect-page-keyboard no;; expect-page-focus no;; clear-line;; keys echo back-\$((3+4))\\r;; wait-screen back-7;; clear-line;; keys m iclk; m iclk; m iclk\\r;; sleep 250;; keys q;; sleep 120;; keys w;; sleep 120;; keys e;; sleep 120;; keys r;; sleep 120;; keys t;; sleep 120;; keys y;; wait-screen % qwerty;; wait-web document.querySelector('#t').value === 'Typed by the agent, key by key, while the user types in the terminal.' => true;; clear-line;; action open-web http://localhost:$WEBPORT/user.html;; wait-browser User Page;; wait-pages 3,4*;; sleep 800;; web-click document.querySelector('#u');; sleep 150;; keys hello;; wait-web document.querySelector('#u').value => \"hello\";; expect-page-focus front;; expect-looked-at yes;; key cmd+j;; wait-toast No agent is waiting for you;; expect-looked-at yes;; expect-screen-count hello 0;; click pane:2;; sleep 400;; expect-page-focus no;; expect-page-keyboard no;; keys echo in-term-\$((5+6))\\r;; wait-screen in-term-11;; wait-web document.querySelector('#u').value => \"hello\";; keys m ikt2\\r;; sleep 600;; click page-tab:4;; wait-pages 3,4*;; sleep 300;; web-click document.querySelector('#u');; sleep 200;; keys abc;; sleep 200;; keys def;; wait-screen ok-ikt2;; wait-web document.querySelector('#u').value => \"helloabcdef\";; expect-page-focus front;; expect-page-keyboard yes;; click pane:2;; sleep 400;; keys m iget\\r;; wait-screen ok-iget;; expect-file $MCP/iget.out Typed by the agent, key by key, while the user types in the terminal. And more.;; action open-web http://localhost:$WEBPORT/self.html;; wait-browser Self Page;; sleep 800;; click pane:2;; sleep 400;; keys echo self-\$((6+7))\\r;; wait-screen self-13;; wait-web document.querySelector('#s').value => \"\";; expect-page-focus no;; quit"

    # An agent's first browser call after a prompt shows the Browser panel. The user hides it;
    # the agent's next call (no prompt since) leaves it hidden; after the user types into the
    # agent's terminal (a prompt), its next call shows it again.
    call snav browser_navigate "{\"url\":\"localhost:$WEBPORT/agent.html\"}"
    call ssnap browser_snapshot '{}'
    call ssnap2 browser_snapshot '{}'
    call ssnapb browser_snapshot '{}'
    printf 'm() { "%s" mcp claude < "%s/$1.json" > "%s/$1.out"; echo "ok-$1"; }\nm snav\nwhile [ ! -f "%s/go" ]; do sleep 0.2; done\nm ssnap\nwhile [ ! -f "%s/go2" ]; do sleep 0.2; done\nm ssnapb\nread a\nm ssnap2\n' "$HOOK" "$MCP" "$MCP" "$MCP" "$MCP" > "$WORK/agentshow.sh"
    mkdir -p "$WORK/data-browser-agent-show"; printf '{"presets":[{"id":"show","name":"Show","command":"sh %s"}]}\n' "$WORK/agentshow.sh" > "$WORK/data-browser-agent-show/settings.json"
    run browser-agent-show 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action run show;; wait-screen ok-snav;; wait-panel shown;; action show-browser;; wait-panel hidden;; write-file $MCP/go go;; wait-screen ok-ssnap;; sleep 500;; wait-panel hidden;; expect-file $MCP/ssnap.out Page: Agent Page;; keys typed;; sleep 300;; write-file $MCP/go2 go;; wait-screen ok-ssnapb;; sleep 500;; wait-panel hidden;; keys again\\r;; wait-screen ok-ssnap2;; wait-panel shown;; quit"

    # The update dialog (a release from a test feed; Later at launch, then Update in the status
    # bar) covers the page: its web view hides under it (else the page would show over the
    # dialog) and comes back after Later.
    printf '{"tag_name":"v99.0.0","html_url":"https://example.invalid/releases/v99.0.0","published_at":"2026-11-18T00:00:00Z","body":"Test release","assets":[]}\n' > "$WORK/feed.json"
    mkdir -p "$WORK/data-browser-update"; printf '{"check_updates":true}\n' > "$WORK/data-browser-update/settings.json"
    export TERNARYCODE_UPDATE_FEED="file://$WORK/feed.json"
    run browser-update 60 "sleep 1000;; action add-project $P1;; wait-shown update-close;; sleep 500;; key esc;; wait-hidden update-close;; action open-web http://localhost:$WEBPORT/;; wait-browser TC Test Page;; wait-pages 3*;; sleep 500;; expect-web-placed;; click update-button;; wait-shown update-close;; sleep 500;; expect-web-hidden;; shot e2e-browser-update;; click update-close;; wait-hidden update-close;; sleep 500;; expect-web-placed;; quit"
    unset TERNARYCODE_UPDATE_FEED

    # More of the browser: $BROWSER (Python's webbrowser) opens the page beside the terminal; its
    # console's error and warning show as a chip, then in the drawer; under the palette the page
    # is a picture; `open https://…` and `ternarycode browse` go to the same browser; a link to a
    # new window, clicked, opens a tab; a page whose server stops says so, and comes back by
    # itself when it answers again; an agent's own JavaScript is refused unless allowed.
    printf '<!doctype html><title>Console Page</title><h1>Console</h1><script>console.log("hello"); console.warn("slow image"); setTimeout(function () { null.qty; }, 50);</script>\n' > "$SITE/console.html"
    printf '<!doctype html><title>New Window Page</title><a href="/second.html" target="_blank" style="display:block;font:40px sans-serif;padding:30px 20px;height:200px">New window</a>\n' > "$SITE/newwin.html"
    DOWNPORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    call evl browser_evaluate '{"function":"() => 1"}'
    run browser-more 90 "sleep 1000;; action add-project $P1;; sleep 1500;; keys python3 -c 'import webbrowser; webbrowser.open(\"http://localhost:$WEBPORT/console.html\")'\\r;; wait-browser Console Page;; wait-pages 3*;; sleep 1000;; expect-shown browser-console:3;; click browser-console:3;; sleep 400;; expect-shown browser-console-drawer:3;; sleep 300;; expect-web-placed;; shot e2e-browser-console;; action palette;; sleep 800;; expect-web-hidden;; shot e2e-browser-still;; key esc;; sleep 600;; expect-web-placed;; focus-pane 0;; keys clear; open http://localhost:$WEBPORT/second.html\\r;; wait-browser Second;; keys \"\$(dirname \"\$TERNARYCODE_HOOK\")/ternarycode\" browse http://localhost:$WEBPORT/newwin.html\\r;; wait-browser New Window Page;; sleep 800;; mouse down browser:3@0.3,0.08;; mouse up browser:3@0.3,0.08;; wait-tabs 0:2;; wait-pages 3,4*;; wait-browser Second;; click page-tab:3;; wait-pages 3*,4;; sleep 300;; focus-pane 0;; keys \"\$(dirname \"\$TERNARYCODE_HOOK\")/ternarycode\" browse http://localhost:$DOWNPORT/\\r;; wait-pages 3,5*,4;; sleep 2500;; expect-shown browser-down:5;; shot e2e-browser-down;; keys python3 -m http.server $DOWNPORT --bind 127.0.0.1 &\\r;; sleep 4000;; expect-hidden browser-down:5;; wait-browser Directory listing;; keys kill %1; $HOOK mcp claude < $MCP/evl.json > $MCP/evl.out; echo ok-\$((1+1))-evl\\r;; wait-screen ok-2-evl;; expect-file $MCP/evl.out JavaScript in pages is off;; action settings-browser;; sleep 600;; shot e2e-settings-browser;; key esc;; quit"
    # A page's script can't take the keyboard from a terminal (it says it was clicked every 50
    # ms; anyone's script can post that); a real click on the page gives it the keyboard.
    printf '<!doctype html><title>Focus Page</title><h1 style="height:400px">Grab</h1><script>setInterval(function () { try { window.ipc.postMessage("focus"); } catch (e) {} }, 50);</script>\n' > "$SITE/focus.html"
    run browser-focus 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys clear; open http://localhost:$WEBPORT/focus.html\\r;; wait-browser Focus Page;; wait-pages 3*;; sleep 1000;; focus-pane 0;; sleep 800;; expect-page-focus no;; keys echo kept-\$((40+2))\\r;; wait-screen kept-42;; expect-page-focus no;; mouse down browser:3@0.5,0.3;; mouse up browser:3@0.5,0.3;; wait-page-focus;; quit"
    # Full screen: each page's own window (browser_host) goes into the window's Space with it,
    # laid over its pane, and the user can type in it there.
    printf '<!doctype html><title>Full Page</title><body style="margin:0"><input id="f" style="position:absolute;left:10px;top:10px;width:200px"></body>\n' > "$SITE/full.html"
    run browser-fullscreen 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys clear; open http://localhost:$WEBPORT/full.html\\r;; wait-browser Full Page;; wait-pages 3*;; sleep 800;; expect-web-placed;; fullscreen on;; sleep 3000;; expect-web-placed;; web-click document.querySelector('#f');; sleep 200;; keys full;; wait-web document.querySelector('#f').value => \"full\";; expect-looked-at yes;; click pane:2;; sleep 300;; keys echo fs-\$((2+3))\\r;; wait-screen fs-5;; fullscreen off;; sleep 3000;; expect-web-placed;; quit"
    # A sign-in pop-up (window.open with a size, after a click in the page, as "Continue with
    # Google" does) opens in a window of its own, tied to the page: it reports back through
    # window.opener and closes itself, and the page goes on signed in, with no new page in the
    # panel. Its title bar names the site; keys with ⌘ stay in it (⌘A selects its field's text, ⌘←
    # goes to the field's start, ⌘K does not open the palette behind it, ⌘W closes the pop-up, not the pane behind it).
    printf '<!doctype html><title>Popup Opener</title><button id="signin" style="font:30px sans-serif;margin:40px;padding:20px">Sign in</button><button id="stay" style="font:30px sans-serif;margin:40px;padding:20px">Stay</button><script>document.getElementById("signin").onclick = function () { window.open("/signin.html", "signin", "width=480,height=560"); }; document.getElementById("stay").onclick = function () { window.open("/signin.html?stay", "stay", "width=480,height=560"); }; window.addEventListener("message", function (e) { if (e.origin === location.origin) document.title = "Signed in: " + e.data; });</script>\n' > "$SITE/popup.html"
    printf '<!doctype html><title>Sign In Page</title><h1>Sign in</h1><input id="f" autofocus oninput="document.title = &quot;Typed: &quot; + this.value"><script>if (location.search !== "?stay") setTimeout(function () { window.opener.postMessage("tc-ok", "*"); window.close(); }, 1500);</script>\n' > "$SITE/signin.html"
    run browser-popup 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys clear; open http://localhost:$WEBPORT/popup.html\\r;; wait-browser Popup Opener;; wait-pages 3*;; sleep 1000;; web-click document.getElementById('signin');; wait-popups 1;; wait-popup signin.html;; sleep 400;; popup-shot e2e-popup;; shot e2e-popup-app;; wait-popups 0;; wait-browser Signed in: tc-ok;; wait-pages 3*;; web-click document.getElementById('stay');; wait-popups 1;; wait-popup stay;; wait-popup-title localhost;; sleep 500;; popup-keys hello;; popup-key cmd+a;; popup-keys z;; wait-popup Typed: z;; popup-key cmd+left;; popup-keys X;; wait-popup Typed: Xz;; popup-key cmd+k;; sleep 600;; expect-web-placed;; wait-popups 1;; popup-key cmd+w;; wait-popups 0;; wait-pages 3*;; wait-tabs 0:2;; quit"
    # The Grid: an agent's page in a panel left hidden (Settings: only the Browser button lights
    # up) shows as a picture in the corner of the agent's tile; a click on it shows the panel in
    # the tile, beside the terminal, where its web view is laid (zoomed out as need be).
    mkdir -p "$WORK/data-browser-grid"; printf '{"browser_agent_beside":false}\n' > "$WORK/data-browser-grid/settings.json"
    call hold2 browser_wait_for '{"text":"never shows","time":15}'
    run browser-grid 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys $HOOK mcp claude < $MCP/nav.json > $MCP/nav2.out; $HOOK mcp claude < $MCP/hold2.json > $MCP/hold2.out &\\r;; wait-tabs 0:2;; wait-pages 3*;; wait-panel hidden;; action grid;; sleep 2000;; expect-shown tile-pip:1;; shot e2e-browser-pip;; click tile-pip:1;; sleep 1200;; expect-shown tile-browser:3;; expect-hidden tile-pip:1;; expect-web-placed;; shot e2e-browser-tile;; quit"
    # The Browser button in the top bar: with a dev server announced, it opens the page in the
    # Browser panel beside the terminal; ⌘L gives its address field what is typed; ⌘R reloads the
    # page; the panel stays in another tab of the project; the button hides it and shows it again;
    # ⌘L in a project with no page opens one, its address field taking the keyboard.
    run browser-button 60 "sleep 1000;; action add-project $P1;; sleep 1500;; expect-shown browser-button;; keys $WORK/bin/announce $WEBPORT\\r;; wait-shown server-chip;; click browser-button;; wait-tabs 0:2;; wait-pages 3*;; wait-browser TC Test Page;; sleep 500;; expect-web-placed;; shot e2e-browser-button;; action browser-address;; sleep 300;; keys localhost:$WEBPORT/second.html\\r;; wait-browser Second;; action browser-reload;; sleep 800;; wait-browser Second;; action new-tab;; sleep 800;; wait-tabs 0:2 1:4;; wait-panel shown;; sleep 300;; expect-web-placed;; click browser-button;; wait-panel hidden;; sleep 400;; expect-web-hidden;; click browser-button;; wait-panel shown;; sleep 500;; expect-web-placed;; wait-browser Second;; action add-project $P2;; sleep 1500;; action browser-address;; sleep 500;; keys localhost:$WEBPORT\\r;; wait-browser TC Test Page;; quit"
    # Splits are resized by their dividers; the Browser panel by its edge, which takes the drag
    # along both its sides (the terminal's and the page's), also when the pointer then crosses
    # the page. The web view follows each time. The Code drawer takes the panel's place, and
    # gives it back.
    D_DRAG='mouse down divider:0@0.5,0.5;; sleep 100;; mouse drag divider:0@'
    D_UP=',0.5;; sleep 150;; mouse up divider:0@0.5,0.5;; sleep 600'
    E_DRAG='mouse down panel-edge@0.5,0.5;; sleep 100;; mouse drag panel-edge@'
    E_UP=',0.5;; sleep 150;; mouse up panel-edge@0.5,0.5;; sleep 600'
    run split-resize 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action split-right;; sleep 1000;; wait-tabs 0:2,3;; ${D_DRAG}-20${D_UP};; expect-width pane:3 650 760;; action close-pane;; sleep 800;; action open-web http://localhost:$WEBPORT/;; wait-browser TC Test Page;; wait-pages 4*;; wait-panel shown;; sleep 800;; expect-width browser:4 440 490;; ${E_DRAG}-30${E_UP};; expect-width browser:4 700 800;; expect-web-placed;; ${E_DRAG}40${E_UP};; expect-width browser:4 418 428;; expect-web-placed;; action editor;; sleep 800;; wait-panel hidden;; expect-web-hidden;; action editor;; sleep 800;; wait-panel shown;; expect-web-placed;; shot e2e-split-resize;; quit"
    # Two agents in one tab, side by side (the case the panel is for): each gets its own page in
    # the one panel; the terminals show stacked beside it (and say so once), side by side again
    # when it hides; the page in front follows the terminal picked, a page tab too; the panel
    # fills the window and Esc puts it back; with four terminals it stays beside them, stacked.
    run browser-panel 90 "sleep 1000;; action add-project $P1;; sleep 1500;; action split-right;; sleep 1000;; wait-tabs 0:2,3;; focus-pane 0;; sleep 300;; keys $HOOK mcp claude < $MCP/nav.json > $MCP/pa.out; echo ok-\$((1+1))a\\r;; wait-screen ok-2a;; wait-pages 4*;; wait-panel stacked;; wait-toast Terminals stacked;; expect-width pane:2 500 620;; expect-width pane:3 500 620;; sleep 500;; expect-web-placed;; shot e2e-browser-panel;; focus-pane 1;; sleep 300;; keys $HOOK mcp claude < $MCP/nav.json > $MCP/pb.out; echo ok-\$((1+1))b\\r;; wait-screen ok-2b;; wait-pages 4,5*;; focus-pane 0;; wait-pages 4*,5;; focus-pane 1;; wait-pages 4,5*;; click page-tab:4;; wait-pages 4*,5;; sleep 400;; expect-web-placed;; click panel-expand;; wait-panel max;; sleep 400;; expect-web-placed;; key esc;; wait-panel stacked;; click panel-close;; wait-panel hidden;; sleep 400;; expect-web-hidden;; expect-width pane:2 420 530;; action show-browser;; wait-panel stacked;; action split-down;; sleep 800;; action split-down;; sleep 800;; wait-panel stacked;; sleep 500;; expect-web-placed;; shot e2e-browser-four;; click page-close:4;; wait-pages 5*;; quit"
    # OpenCode gets the browser tools in every pane: its inline config names the server and the
    # guide file, written in the app's data folder.
    run opencode-env 40 "sleep 1000;; action add-project $P1;; sleep 1500;; keys python3 -c 'import json,os; c=json.loads(os.environ[\"OPENCODE_CONFIG_CONTENT\"]); print(\"oc-\"+c[\"mcp\"][\"ternarycode\"][\"command\"][2]+\"-\"+str(os.path.exists(c[\"instructions\"][-1])))'\\r;; wait-screen oc-opencode-True;; quit"
    # A workspace of an older version, its pages in tabs: they move to the project's panel (the
    # page's own tab goes), shown since one was in the tab in view; saved so from then on.
    mkdir -p "$WORK/data-browser-migrate"
    printf '{"version":1,"projects":[{"id":1,"name":"alpha","path":"%s","color":0,"tabs":[{"layout":{"split":{"dir":"horizontal","ratio":0.5,"a":{"pane":2},"b":{"pane":3}}},"focused":3},{"layout":{"pane":4},"focused":4}],"active_tab":0,"panes":[{"id":2,"cwd":"%s"},{"id":3,"browser":"http://localhost:%s/second.html"},{"id":4,"browser":"http://localhost:%s/"}]}],"active":1,"next_id":5}\n' "$P1" "$P1" "$WEBPORT" "$WEBPORT" > "$WORK/data-browser-migrate/workspace.json"
    run browser-migrate 40 "sleep 2000;; expect-project alpha;; wait-tabs 0:2;; wait-pages 3,4*;; wait-panel shown;; wait-browser TC Test Page;; sleep 500;; expect-web-placed;; click page-tab:3;; wait-browser Second;; quit"
    [ -z "$ONLY" ] || [[ "browser-migrate" == *"$ONLY"* ]] && expect_saved browser-migrate browser_panel
    kill "$WEB_PID" 2>/dev/null
fi

run foreground 40 "sleep 1000;; action add-project $P1;; sleep 1200;; action split-right;; sleep 1000;; focus-pane 0;; sleep 500;; keys sleep 30\\r;; wait-status running;; key ctrl+c;; sleep 500;; wait-status idle;; quit"

run hidden 40 "sleep 1000;; action add-project $P1;; sleep 1000;; action add-project $P2;; sleep 1000;; close-window;; sleep 1500;; expect-alive 2;; hook 2 claude Notification permission_prompt;; sleep 800;; wait-queue 1;; quit"

run quit-restore-1 40 "sleep 1000;; action add-project $P1;; sleep 1200;; hook 2 claude SessionStart;; sleep 500;; keys sleep 300\\r;; sleep 1200;; action quit;; sleep 500;; expect-dialog open;; click dialog-ok;; sleep 3000" no-done
mkdir -p "$WORK/data-quit-restore-2"; cp "$WORK/data-quit-restore-1/workspace.json" "$WORK/data-quit-restore-2/" 2>/dev/null
run quit-restore-2 40 "sleep 2000;; expect-project alpha;; expect-rect resume:2;; action grid;; action grid-active;; sleep 800;; expect-shown tile:alpha;; expect-shown tile-resume:2;; action grid-all;; quit"

# An agent to resume in the second tab of a project, the other tab last open (the keyboard in
# beta): its Grid tile shows that tab with the banner, the project is Active for it; Dismiss
# takes it out of Active, and the next restart does not offer it again.
run grid-resume-1 40 "sleep 1000;; action add-project $P1;; sleep 1200;; hook 2 claude SessionStart;; sleep 500;; action new-tab;; sleep 800;; wait-tabs 0:2 1:3;; action add-project $P2;; sleep 1200;; action quit;; sleep 3000" no-done
mkdir -p "$WORK/data-grid-resume-2"; cp "$WORK/data-grid-resume-1/workspace.json" "$WORK/data-grid-resume-2/" 2>/dev/null
run grid-resume-2 40 "sleep 2000;; action grid;; action grid-active;; sleep 1000;; expect-shown tile:alpha;; expect-tile-tab alpha 0;; expect-shown tile-resume:2;; expect-shown tile-tab:alpha:1;; shot e2e-grid-resume;; click tile-dismiss:2;; sleep 800;; expect-hidden tile:alpha;; action quit;; sleep 3000" no-done
mkdir -p "$WORK/data-grid-resume-3"; cp "$WORK/data-grid-resume-2/workspace.json" "$WORK/data-grid-resume-3/" 2>/dev/null
run grid-resume-3 40 "sleep 2000;; action grid;; action grid-active;; sleep 1000;; expect-hidden tile:alpha;; action select-name alpha;; sleep 500;; action select-tab 0;; sleep 800;; expect-project alpha;; expect-tab 0;; expect-shown pane:2;; expect-hidden resume:2;; quit"

# Restored panes sleep until shown (v1.2): a restored Grid of six projects shows 2 × 2 and starts
# only those four shells; the two below the fold start when scrolled to or opened.
G="$WORK/grid6"; for i in 1 2 3 4 5 6; do mkdir -p "$G/g$i"; done
run lazy-1 50 "sleep 1000;; action add-project $G/g1;; sleep 700;; action add-project $G/g2;; sleep 700;; action add-project $G/g3;; sleep 700;; action add-project $G/g4;; sleep 700;; action add-project $G/g5;; sleep 700;; action add-project $G/g6;; sleep 1200;; expect-started 6;; action grid;; sleep 800;; quit"
mkdir -p "$WORK/data-lazy-2"; cp "$WORK/data-lazy-1/workspace.json" "$WORK/data-lazy-2/" 2>/dev/null
# The wheel over the tile taking the keyboard scrolls its terminal, not the Grid; over another
# tile it scrolls the Grid, and the tiles scrolled into view start their shells.
# Each restored terminal fills its tile; opened from the Grid, it takes its Focus size.
run lazy-2 40 "sleep 2500;; expect-started 4;; expect-shown tile:g1;; expect-shown tile:g4;; expect-hidden tile:g5;; expect-hidden tile:g6;; expect-fill 2;; expect-fill 8;; shot e2e-grid-2x2;; click tile:g1;; sleep 300;; scroll tile:g1 -1000;; sleep 800;; expect-hidden tile:g5;; scroll tile:g2 -1000;; sleep 1000;; expect-shown tile:g5;; expect-shown tile:g6;; expect-started 6;; expect-typing g1;; expect-fill 12;; shot e2e-grid-scrolled;; sleep 600;; action select 5;; sleep 1200;; expect-fill 12;; quit"

# A split closed from the Grid (Ctrl+D in a tile): the pane left fills the whole tile, and
# opening the project gives it its Focus size.
# Documents in the editor: a PDF renders its pages, a CSV and a Word file read into a table and
# text, a picture shows; a CSV can be edited as text.
DOCS="$WORK/docs"; mkdir -p "$DOCS"
printf 'name,qty,code\nalpha,3,007\nbeta,4.5,\n' > "$DOCS/data.csv"
python3 - "$DOCS" <<'PY'
import sys, zipfile
d = sys.argv[1]
res = "/Resources << /Font << /F1 6 0 R >> >>"
objs = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R %s >>" % res,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R %s >>" % res]
body = b"0 0 1 rg 100 100 200 200 re f 0 0 0 rg BT /F1 24 Tf 72 700 Td (Quarterly Report) Tj ET"
objs.append("<< /Length %d >>\nstream\n%s\nendstream" % (len(body), body.decode()))
objs.append("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
out, offs = b"%PDF-1.4\n", []
for i, o in enumerate(objs):
    offs.append(len(out)); out += ("%d 0 obj\n%s\nendobj\n" % (i + 1, o)).encode()
x = len(out)
out += ("xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)).encode()
out += b"".join(("%010d 00000 n \n" % o).encode() for o in offs)
out += ("trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, x)).encode()
open(d + "/report.pdf", "wb").write(out)
w = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
with zipfile.ZipFile(d + "/notes.docx", "w") as z:
    z.writestr("word/document.xml", '<w:document %s><w:body><w:p><w:r><w:t>Hello docs</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>' % w)
import struct, zlib
def chunk(t, data):
    return struct.pack(">I", len(data)) + t + data + struct.pack(">I", zlib.crc32(t + data) & 0xffffffff)
raw = b"\x00" + b"\xff\x00\x00\xff" * 2
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 1, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
open(d + "/dot.png", "wb").write(png)
PY
run docs 60 "sleep 1000;; action add-project $DOCS;; sleep 1200;; open-file $DOCS/report.pdf;; wait-doc kind=pdf state=ready pages=2;; wait-doc rendered>0;; shot e2e-doc-pdf;; open-file $DOCS/data.csv;; wait-doc kind=csv state=ready rows=3 cols=3;; open-file $DOCS/notes.docx;; wait-doc kind=word state=ready blocks=2;; open-file $DOCS/dot.png;; wait-doc kind=image pictures=1;; open-file $DOCS/data.csv;; wait-doc rows=3;; quit"
# ⌘F in a document (after a click in it took the keyboard from the terminal): a PDF's text on
# both pages, then Enter to the second; a sheet's cell (selected); a Word file's words.
run doc-find 60 "sleep 1000;; action add-project $DOCS;; sleep 1200;; open-file $DOCS/report.pdf;; wait-doc kind=pdf state=ready pages=2;; click doc-body;; sleep 200;; action find;; sleep 300;; keys quarterly;; wait-doc matches=2 current=1;; key enter;; wait-doc current=2;; shot e2e-doc-find-pdf;; key esc;; sleep 300;; expect-hidden doc-find;; open-file $DOCS/data.csv;; wait-doc kind=csv state=ready;; click doc-body;; action find;; sleep 300;; keys beta;; wait-doc matches=1 cell=2,0;; key esc;; open-file $DOCS/notes.docx;; wait-doc kind=word state=ready;; click doc-body;; action find;; sleep 300;; keys docs;; wait-doc matches=1 current=1;; shot e2e-doc-find-word;; quit"

# The sidebar's Active group: nothing running, no groups; a program running in beta for 2 s
# moves it under Active, above alpha, and ⌘1 follows; Idle folds; while the pointer is over the
# list nothing moves, once it leaves beta goes back (Active lingers 0.6 s here, 30 s normally).
export TERNARYCODE_ACTIVE_LINGER_MS=600
run sidebar-order 60 "sleep 1000;; action add-project $P1;; sleep 1000;; action add-project $P2;; sleep 1200;; expect-above project:alpha project:beta;; expect-project beta;; key cmd+alt+up;; sleep 600;; expect-above project:beta project:alpha;; action select 0;; sleep 400;; expect-project beta;; keys sleep 5\\r;; sleep 3000;; expect-above project:beta project:alpha;; key ctrl+c;; action select 1;; sleep 400;; expect-project alpha;; action move-up;; sleep 400;; expect-above project:alpha project:beta;; shot e2e-sidebar-order;; quit"
unset TERNARYCODE_ACTIVE_LINGER_MS

# The sidebar keeps the width it is dragged to (it used to shrink back to its narrowest).
run sidebar-resize 40 "sleep 1000;; action add-project $P1;; sleep 1200;; expect-width sidebar 240 256;; mouse down sidebar@1.0,0.5;; sleep 100;; mouse drag sidebar@1.4,0.5;; sleep 150;; mouse up sidebar@1.0,0.5;; sleep 1500;; expect-width sidebar 335 360;; sleep 1500;; expect-width sidebar 335 360;; quit"
run grid-fit 50 "sleep 1000;; action add-project $P1;; sleep 1200;; action split-right;; sleep 1500;; wait-tabs 0:2,3;; action grid;; sleep 1000;; expect-fill 2;; expect-fill 3;; mouse down tile:alpha@0.75,0.5;; mouse up tile:alpha@0.75,0.5;; sleep 300;; key ctrl+d;; wait-tabs 0:2;; sleep 800;; expect-fill 2;; shot e2e-grid-fit;; action focus;; sleep 1000;; expect-fill 2;; quit"

# A zsh prompt survives resizes whole (v1.2): split, close the split, open and close the editor;
# the screen keeps one prompt per command and no piece of an old one (shell integration marks
# the prompt, the resize clears it before the reflow, zsh redraws it).
run prompt-resize 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys echo hi\\r;; wait-screen hi;; sleep 600;; action split-right;; sleep 1500;; action close-pane;; sleep 1500;; action editor;; sleep 1500;; action editor;; sleep 1500;; expect-screen-count alpha % 2;; expect-no-line %;; shot e2e-prompt-resize;; quit"

# The editor's language features (v1.3), with a language server answering from plain text.
# Opening a file starts no server (servers on demand): go to definition starts it and runs once
# it is ready. Then: definition (another file) and back, references, diagnostics, completion, rename across
# files (on disk) and its undo, a code action, formatting.
L="$WORK/lsp"; mkdir -p "$L/src"
printf 'fn main() {\n    let x = helper(1);\n    println!("{x}");\n}\n' > "$L/src/main.rs"
printf 'pub fn helper(a: i32) -> i32 {\n    a + 1\n}\n\n// BUG here\n' > "$L/src/util.rs"
run lsp 90 "sleep 1000;; action add-project $L;; sleep 1200;; open-file $L/src/main.rs;; sleep 800;; expect-no-lsp;; goto $L/src/main.rs 1 14;; sleep 400;; action ide def;; wait-active util.rs;; wait-caret 1;; wait-diag 1 0;; action ide back;; wait-active main.rs;; wait-caret 2;; action ide refs;; wait-locations 2;; key esc;; sleep 300;; action ide rename;; wait-rename;; sleep 300;; keys helper2\\r;; sleep 1200;; expect-file $L/src/util.rs fn helper2(;; expect-file $L/src/main.rs helper2(1);; action ide undo;; sleep 800;; expect-file $L/src/util.rs fn helper(;; expect-file $L/src/main.rs helper(1);; goto $L/src/main.rs 2 4;; sleep 400;; keys al;; wait-completion;; key enter;; wait-editor alpha_item;; action ide actions;; sleep 600;; action ide run-action;; wait-editor // TODO;; shot e2e-lsp;; quit"
# Typing while the server works on a rename: its answer is for the old text, so nothing is
# applied (the file not open in the editor keeps its name on disk too).
S="$WORK/lsp-stale"; mkdir -p "$S/src"
printf 'fn main() {\n    let x = helper(1);\n    println!("{x}");\n}\n' > "$S/src/main.rs"
printf 'pub fn helper(a: i32) -> i32 {\n    a + 1\n}\n' > "$S/src/util.rs"
TC_FAKE_LSP_DELAY_MS=1500 run lsp-stale 60 "sleep 1000;; action add-project $S;; sleep 1200;; open-file $S/src/main.rs;; goto $S/src/main.rs 1 14;; sleep 400;; action ide rename;; wait-lsp ready;; wait-rename;; sleep 300;; keys helper2\\r;; sleep 300;; keys zz;; wait-editor zz;; wait-toast nothing was applied|tidak ada yang diterapkan;; expect-file $S/src/util.rs fn helper(;; expect-not-file $S/src/util.rs helper2;; quit"
# ⌘-click on a symbol goes to its definition (starting the server: hovering alone starts none);
# then the pointer resting on one shows its hover.
run lsp-click 60 "sleep 1000;; action add-project $L;; sleep 1200;; open-file $L/src/main.rs;; goto $L/src/main.rs 1 14;; sleep 500;; mouse move editor-caret;; sleep 1200;; expect-no-lsp;; mouse cmd-move editor-caret;; sleep 200;; mouse cmd-down editor-caret;; mouse cmd-up editor-caret;; wait-active util.rs;; wait-caret 1;; action ide back;; wait-active main.rs;; goto $L/src/main.rs 1 14;; sleep 500;; mouse move editor-caret;; wait-hover;; shot e2e-lsp-click;; quit"
# Without the server installed: the status line says so, with how to install it.
LSP=/nonexistent/tc-no-lsp run lsp-missing 40 "sleep 1000;; action add-project $L;; sleep 1200;; open-file $L/src/util.rs;; action ide def;; wait-lsp missing;; expect-rect ide-status;; quit"

# Grid filter (v1.2): "Active" keeps only projects running something or waiting.
run grid-active 50 "sleep 1000;; action add-project $P1;; sleep 900;; action add-project $P2;; sleep 1200;; keys sleep 30\\r;; wait-status running;; sleep 2500;; action grid;; sleep 800;; expect-shown tile:beta;; expect-shown tile:alpha;; click grid:active;; sleep 600;; expect-shown tile:beta;; expect-hidden tile:alpha;; shot e2e-grid-active;; click grid:all;; sleep 600;; expect-shown tile:alpha;; quit"

run grid 60 "sleep 1000;; action add-project $P1;; sleep 900;; action split-right;; sleep 800;; keys echo alpha-left\\r;; action add-project $P2;; sleep 900;; action split-down;; sleep 800;; action add-project $R;; sleep 1200;; action grid;; sleep 1200;; expect-fill 2;; expect-fill 3;; expect-fill 5;; expect-fill 6;; expect-fill 8;; shot e2e-grid;; action focus;; sleep 1200;; expect-fill 8;; action select 0;; sleep 1200;; expect-fill 2;; expect-fill 3;; wait-screen alpha-left;; click nav:overview;; sleep 1200;; expect-shown tile:beta;; shot e2e-all-projects;; click nav:overview;; sleep 1200;; expect-hidden tile:beta;; click nav:overview;; sleep 1200;; expect-shown tile:beta;; click project:beta;; sleep 1200;; expect-project beta;; expect-hidden tile:alpha;; shot e2e-focus-beta;; quit"

run editor 60 "sleep 1000;; action add-project $P1;; sleep 1200;; open-file $P1/notes.txt;; sleep 800;; write-file $P1/notes.txt notes v2 by agent\\n;; wait-editor v2 by agent;; open-file $P1/src/main.rs:2;; sleep 800;; click editor-text;; sleep 300;; key cmd+down;; keys // mine\\r;; wait-editor // mine;; write-file $P1/src/main.rs fn main() { agent(); }\\n;; wait-conflict;; expect-not-file $P1/src/main.rs // mine;; action force-save;; sleep 500;; expect-file $P1/src/main.rs // mine;; quit"

# The Code drawer's Files column (v1.1, in the drawer since v0.2 of the design): click a file → the editor shows it; files an agent adds or
# changes appear (and get their git letter) without a refresh; nothing is read per frame.
# Review follows the project in view: picking another project while Review is open shows that
# project's changes; picking the first again shows its own.
FA="$WORK/follow-a"; FB="$WORK/follow-b"
for d in "$FA" "$FB"; do mkdir -p "$d" && (cd "$d" && git init -q -b main && printf 'x\n' > keep.txt && git add -A && git -c user.name=t -c user.email=t@t commit -qm init); done
printf 'a\n' > "$FA/only-a.txt"; printf 'b\n' > "$FB/only-b.txt"
run review-follow 60 "sleep 1000;; action add-project $FA;; sleep 1000;; action add-project $FB;; sleep 1000;; action select-name follow-a;; sleep 600;; action review;; wait-review Added:only-a.txt;; action select-name follow-b;; wait-review Added:only-b.txt;; expect-project follow-b;; action select-name follow-a;; wait-review Added:only-a.txt;; quit"

T="$WORK/tree"; mkdir -p "$T/src"
(cd "$T" && git init -q -b main && printf 'line 1\n' > edit.txt && printf 'fn main() {}\n' > src/main.rs \
    && git add -A && git -c user.name=t -c user.email=t@t commit -qm init)
run explorer 60 "sleep 1000;; action add-project $T;; sleep 1500;; action editor;; sleep 600;; wait-tree edit.txt;; expect-shown tree:edit.txt;; action files;; sleep 400;; expect-hidden tree:edit.txt;; action files;; sleep 400;; expect-shown tree:edit.txt;; click tree:edit.txt;; wait-open edit.txt;; expect-rect editor-text;; write-file $T/edit.txt line 1 changed by agent\\n;; wait-git edit.txt M;; write-file $T/added.txt new\\n;; wait-tree added.txt;; wait-git added.txt U;; click panel-close;; sleep 400;; expect-hidden panel;; click panel-toggle:Files;; wait-open edit.txt;; shot e2e-explorer;; quit"


# Terminal tabs (v1.1): drag a tab along the strip to reorder; drop it on a pane's right half to
# make it a split of the tab in view. Ctrl+D in a shell closes its pane.
run tabs 60 "sleep 1000;; action add-project $P2;; sleep 1200;; action new-tab;; sleep 800;; action new-tab;; sleep 800;; wait-tabs 0:2 1:3 2:4;; mouse down tab:2;; sleep 120;; mouse drag tab:2@0.3,0.5;; sleep 120;; mouse drag tab:1@0.5,0.5;; sleep 120;; mouse drag tab:0@0.2,0.5;; sleep 150;; mouse up tab:0@0.2,0.5;; sleep 400;; wait-tabs 0:4 1:2 2:3;; mouse down tab:1;; sleep 120;; mouse drag tab:1@0.5,0.9;; sleep 120;; mouse drag pane:4@0.7,0.5;; sleep 120;; mouse drag pane:4@0.92,0.5;; sleep 150;; shot e2e-tab-drop;; mouse up pane:4@0.92,0.5;; sleep 500;; wait-tabs 0:4,2 1:3;; quit"

# The open tab dragged onto its own pane: the tab looked at before it shows, and the open tab is
# dropped beside it. Then the merged tab, held over the first tab in the strip, makes that one
# show (spring-loaded), and is dropped beside its pane.
run tab-open-split 60 "sleep 1000;; action add-project $P2;; sleep 1200;; action new-tab;; sleep 800;; action new-tab;; sleep 800;; wait-tabs 0:2 1:3 2:4;; mouse down tab:2;; sleep 120;; mouse drag tab:2@0.5,0.9;; sleep 120;; mouse drag pane:4@0.5,0.5;; sleep 300;; mouse drag pane:3@0.7,0.5;; sleep 120;; mouse drag pane:3@0.92,0.5;; sleep 150;; shot e2e-tab-open-split;; mouse up pane:3@0.92,0.5;; sleep 500;; wait-tabs 0:2 1:3,4;; mouse down tab:1;; sleep 120;; mouse drag tab:1@0.6,0.5;; sleep 120;; mouse drag tab:0@0.5,0.5;; sleep 700;; mouse drag tab:0@0.5,0.9;; sleep 120;; mouse drag pane:2@0.7,0.5;; sleep 120;; mouse drag pane:2@0.92,0.5;; sleep 150;; mouse up pane:2@0.92,0.5;; sleep 500;; wait-tabs 0:2,3,4;; quit"

# ⌘F with the keyboard in the editor searches the file (all matches found, Enter steps); `:N`
# goes to line N. ⌘F in a terminal still searches the pane (scenario `find`).
run editor-find 50 "sleep 1000;; action add-project $P1;; sleep 1200;; write-file $P1/find.txt one two one\\nthree\\nfour one\\n;; open-file $P1/find.txt;; sleep 800;; click editor-text;; sleep 300;; key cmd+f;; sleep 300;; keys one;; wait-find 3;; key enter;; sleep 200;; key cmd+f;; sleep 300;; keys :3\\r;; wait-caret 3;; sleep 300;; wait-editor one two one\\nthree\\nfour one\\n;; quit"
# A pane header dragged onto another pane's lower half moves the pane there. A pane made by a
# split starts at its final size: its PTY is never resized afterwards.
run pane-drag 50 "sleep 1000;; action add-project $P2;; sleep 1200;; action split-right;; sleep 1500;; wait-tabs 0:2,3;; expect-resizes 3 0;; mouse down pane-header:2@0.3,0.5;; sleep 120;; mouse drag pane-header:2@0.4,0.6;; sleep 120;; mouse drag pane:3@0.5,0.7;; sleep 120;; mouse drag pane:3@0.5,0.92;; sleep 150;; mouse up pane:3@0.5,0.92;; sleep 500;; wait-tabs 0:3,2;; quit"

# The file tree's own actions: a new file (named in place, then opened), renamed in place (the
# open file follows), and a nested folder.
F="$WORK/files"; mkdir -p "$F"; printf 'x\n' > "$F/a.txt"
# Esc in the files filter empties it (the whole tree is back).
run explorer-filter-esc 40 "sleep 1000;; action add-project $T;; sleep 1500;; wait-tree edit.txt;; click files-filter;; sleep 300;; keys zzz;; sleep 500;; expect-hidden tree:edit.txt;; key esc;; sleep 500;; expect-shown tree:edit.txt;; quit"
run files-actions 50 "sleep 1000;; action add-project $F;; sleep 1500;; wait-tree a.txt;; sleep 300;; click files-button:0;; sleep 400;; keys hello.txt\\r;; wait-open hello.txt;; wait-tree hello.txt;; action rename hello.txt;; sleep 400;; keys renamed\\r;; wait-tree renamed.txt;; wait-open renamed.txt;; click files-button:1;; sleep 400;; keys docs/api\\r;; wait-tree docs;; shot e2e-files;; quit"
run ctrl-d 40 "sleep 1000;; action add-project $P1;; sleep 1200;; action split-right;; sleep 2600;; wait-tabs 0:2,3;; key ctrl+d;; wait-tabs 0:2;; sleep 300;; key ctrl+d;; wait-tabs 0:4;; sleep 600;; key ctrl+d;; wait-tabs 0:5;; quit"
# Review opened from "N changed" shows those N, since the last commit, in the Code drawer (not a
# page of its own). A new folder made before the agent started: the agent's own run has nothing,
# so Review from the agent falls back to the last commit; asked for the run, it says why it is
# empty and offers the changes since the last commit. Full screen and back with Esc.
RS="$WORK/scope"; mkdir -p "$RS" && (cd "$RS" && git init -q -b main && printf 'one\n' > a.txt && git add -A && git -c user.name=t -c user.email=t@t commit -qm init && mkdir -p notes && printf 'x\n' > notes/n1.md && printf 'y\n' > notes/n2.md)
run review-scope 70 "sleep 1000;; action add-project $RS;; sleep 1500;; hook 2 claude SessionStart;; sleep 300;; wait-snapshot;; action review;; wait-review Added:notes/n1.md,Added:notes/n2.md;; expect-review Head drawer;; expect-shown review-dir:notes;; action review-agent;; wait-review Added:notes/n1.md,Added:notes/n2.md;; expect-review Head drawer;; click review-base:Run;; sleep 800;; wait-review ;; expect-review Run drawer;; expect-shown review-show-head;; shot e2e-review-scope-empty;; click review-show-head;; wait-review Added:notes/n1.md,Added:notes/n2.md;; expect-review Head drawer;; shot e2e-review-drawer;; quit"

run review 60 "sleep 1000;; action add-project $R;; sleep 1200;; hook 2 claude SessionStart;; sleep 300;; wait-snapshot;; write-file $R/edit.txt line 1\\nline 2 changed\\nline 3\\n;; write-file $R/new.txt brand new\\n;; rm-file $R/gone.txt;; sleep 300;; action review;; sleep 1500;; wait-review Added:new.txt,Deleted:gone.txt,Modified:edit.txt;; quit"
# An agent that finished while another project was open: Review in the list of what waits, and
# on its Grid tile, opens what it changed.
run review-done 70 "sleep 1000;; action add-project $R;; sleep 1200;; hook 2 claude SessionStart;; sleep 300;; wait-snapshot;; write-file $R/done.txt finished\\n;; action add-project $P1;; sleep 1200;; hook 2 claude Stop;; sleep 600;; action attention;; sleep 600;; click att-review:2;; sleep 1500;; wait-review Added:done.txt;; expect-project repo;; key esc;; sleep 600;; action select-name alpha;; sleep 800;; hook 2 claude Stop;; sleep 600;; action grid;; sleep 1200;; click tile-review:2;; sleep 1500;; wait-review Added:done.txt;; expect-project repo;; shot e2e-review-done;; quit"

# Shell integration (OSC 133) with the user's own shell configuration: a quick failure stays
# quiet; a long one is reported as failed with its exit code.
run shell-integration 60 "sleep 1000;; action add-project $P1;; sleep 2500;; keys false\\r;; sleep 1000;; wait-status idle;; keys sleep 11; false\\r;; sleep 12500;; wait-status failed;; quit"

run find 40 "sleep 1000;; action add-project $P1;; sleep 1200;; keys seq 1 300\\r;; sleep 800;; action find;; sleep 400;; keys 150;; key enter;; sleep 400;; key esc;; action find-all;; sleep 400;; keys 27;; sleep 800;; key esc;; quit"

# Worktrees: Run Agent → New worktree makes one beside the repository (its ignored .env copied)
# and lists it under the project; Review shows its branch since it left main (a commit and a file
# not committed); Merge types the merge in the project's terminal; once merged, Remove takes the
# folder and the branch away.
W="$WORK/wtrepo"; mkdir -p "$W"
(cd "$W" && git init -q -b main && git config user.name t && git config user.email t@t && printf '.env\n' > .gitignore && printf 'KEY=1\n' > .env && printf 'one\n' > a.txt && git add -A && git commit -qm init)
WT="$WORK/wtrepo.worktrees/feat"
run worktree 90 "sleep 1000;; action add-project $W;; sleep 1500;; action worktree none feat main;; wait-project feat;; sleep 800;; expect-project feat;; expect-shown project:feat;; expect-file $WT/.env KEY=1;; expect-file $WT/a.txt one;; keys echo two >> a.txt && git commit -qam work && echo new > b.txt\\r;; sleep 1500;; action review;; sleep 1500;; wait-review Added:b.txt,Modified:a.txt;; shot e2e-worktree-review;; key esc;; sleep 400;; write-file $WT/b.txt new\\n;; rm-file $WT/b.txt;; sleep 800;; action merge-worktree;; sleep 500;; expect-dialog open;; click dialog-ok;; sleep 400;; expect-project wtrepo;; wait-screen Merge made;; wait-toast Merged tc/feat;; expect-file $W/a.txt two;; action select-name feat;; sleep 600;; action remove-worktree;; sleep 500;; expect-dialog open;; shot e2e-worktree-remove;; click dialog-ok;; sleep 2500;; expect-hidden project:feat;; expect-not-file $WT/a.txt one;; quit"

# The commit box: the message comes from the agent's task and last words; a file left out of
# the commit stays out; Commit & Push sends the rest to the remote (a local bare repository:
# no pages, so no pull request).
SH="$WORK/ship"; mkdir -p "$SH"
(cd "$SH" && git init -q -b main && git config user.name t && git config user.email t@t && printf 'one\n' > a.txt && printf 'keep\n' > b.txt && git add -A && git commit -qm init && git init -q --bare -b main "$WORK/ship-remote.git" && git remote add origin "$WORK/ship-remote.git" && git push -q -u origin main && printf 'two\n' > a.txt && printf 'not this\n' > b.txt)
run ship 60 "sleep 1000;; action add-project $SH;; sleep 1500;; hook 2 claude UserPromptSubmit - ship it from the e2e, then stop;; sleep 300;; hook 2 claude Stop - Changed a.txt only.;; sleep 300;; action review;; wait-review Modified:a.txt,Modified:b.txt;; wait-shown ship-go;; click review-in:b.txt;; sleep 400;; shot e2e-ship;; click ship-go;; wait-shown ship-act-done;; focus-pane 0;; keys git -C $WORK/ship-remote.git log -1 --format=MSG:%s/%b main; git diff --quiet -- b.txt || echo B-LEFT-OUT\\r;; wait-screen MSG:Ship it from the e2e/Changed a.txt only.;; wait-screen B-LEFT-OUT;; quit"

# A Grid tile of a project with two tabs (alpha: the agent's tab 0, a shell in tab 1, the one
# last open; the keyboard is in beta's tile): it shows the tab last open while nothing happens;
# the agent asking brings its tab; a tab picked from the strip holds until something new waits;
# the tile typed into never changes by itself; ↗ opens Focus on the tab shown.
run grid-tabs 60 "sleep 1000;; action add-project $P1;; sleep 1200;; action new-tab;; sleep 800;; wait-tabs 0:2 1:3;; action add-project $P2;; sleep 1200;; action grid;; sleep 1200;; expect-tile-tab alpha 1;; hook 2 claude PermissionRequest;; sleep 800;; expect-tile-tab alpha 0;; expect-shown tile-tab:alpha:1;; shot e2e-grid-tabs;; click tile-tab:alpha:1;; sleep 600;; expect-tile-tab alpha 1;; sleep 1200;; expect-tile-tab alpha 1;; hook 2 claude Stop;; sleep 800;; expect-tile-tab alpha 0;; click tile-tab:alpha:1;; sleep 600;; expect-tile-tab alpha 1;; click tile-pane:3;; sleep 500;; hook 2 claude PermissionRequest;; sleep 800;; expect-tile-tab alpha 1;; click tile-open:alpha;; sleep 800;; expect-project alpha;; expect-tab 1;; quit"

# Ground-up ADE redesign E2E checks:
# 1. Wide Code Editor (>= 60% width in Bottom & Right dock), collapsible drawer tabs,
#    Drawer maximize/restore, and ToggleDrawer when Code Editor hidden.
run panel 70 "sleep 1000;; action add-project $R;; sleep 1200;; write-file $R/edit.txt line 1\\nline 2 changed\\nline 3\\n;; sleep 1500;; expect-hidden panel;; expect-shown stage;; action review;; sleep 800;; expect-shown panel;; expect-shown stage;; shot e2e-panel-changes;; action expand-panel;; sleep 500;; expect-shown panel;; expect-hidden stage;; action expand-panel;; sleep 500;; expect-shown stage;; action history;; sleep 800;; expect-shown panel;; action editor;; sleep 800;; expect-shown panel;; action editor;; sleep 500;; expect-hidden panel;; action drawer;; sleep 600;; expect-shown panel;; click panel-close;; sleep 500;; expect-hidden panel;; action review;; sleep 600;; action zen;; sleep 500;; expect-hidden panel;; expect-hidden nav:inbox;; expect-shown stage;; action zen;; sleep 500;; expect-shown panel;; expect-shown nav:inbox;; quit"

# 2. Project-nested agent session history inside the left workspace sidebar tree (collapsed by default + inline fold/unfold).
run sessions 50 "sleep 1000;; action add-project $P1;; sleep 1200;; expect-shown session:1:0;; action add-project $P2;; sleep 1200;; expect-project beta;; expect-shown session:3:0;; expect-hidden session:1:0;; action new-tab;; sleep 800;; expect-shown session:3:1;; expect-tab 1;; click session:3:0;; sleep 500;; expect-tab 0;; shot e2e-sessions;; click project:alpha;; sleep 600;; expect-project alpha;; expect-shown session:1:0;; expect-hidden session:3:0;; quit"

# 2b. Sidebar scroll stability with 20 projects (with expanded sessions!) — verifies zero infinite scroll jump.
SCROLL_SCRIPT="sleep 800"
for i in $(seq -w 1 20); do
    PD="$WORK/proj-$i"
    mkdir -p "$PD"
    # Seed a fake Claude session under each project so expanding allocates real session rows!
    SLUG=$(python3 -c 'import os,re,sys; p=os.path.realpath(sys.argv[1]); print(re.sub(r"[^A-Za-z0-9]","-",p))' "$PD")
    mkdir -p "$TERNARYCODE_AGENT_HOME/.claude/projects/$SLUG"
    cat > "$TERNARYCODE_AGENT_HOME/.claude/projects/$SLUG/11111111-1111-1111-1111-1111111111${i}.jsonl" <<JSONL
{"type":"user","message":{"content":"work on proj-$i"},"gitBranch":"main"}
{"type":"ai-title","aiTitle":"Session proj-$i"}
JSONL
    SCROLL_SCRIPT="$SCROLL_SCRIPT;; action add-project $PD"
done
SCROLL_SCRIPT="$SCROLL_SCRIPT;; sleep 1000;; action select-name proj-01;; sleep 800;; expect-shown session:1:0;; scroll sidebar@0.5,0.5 -220;; sleep 300;; scroll sidebar@0.5,0.5 -220;; sleep 300;; scroll sidebar@0.5,0.5 -220;; sleep 300;; scroll sidebar@0.5,0.5 -220;; sleep 300;; expect-hidden project:proj-01;; expect-hidden session:1:0;; expect-shown project:proj-20;; shot e2e-ade-scroll;; scroll sidebar@0.5,0.5 440;; sleep 300;; scroll sidebar@0.5,0.5 440;; sleep 300;; expect-shown project:proj-01;; expect-shown session:1:0;; quit"
run ade-sidebar-scroll 90 "$SCROLL_SCRIPT"

# 3. Multi-account Profiles view, Rate-Limit Manual & Auto Failover, and All-Agent Usage Monitor.
run ade-profiles-monitor 60 "sleep 1000;; action add-project $P1;; sleep 1200;; set-failover manual;; trigger-limit claude1;; sleep 500;; expect-shown rate-limit-banner:2;; shot e2e-ade-failover;; set-failover auto;; trigger-limit claude1;; sleep 500;; wait-toast auto-resumed|otomatis dilanjutkan;; action profiles;; sleep 600;; expect-dialog open;; expect-shown settings-body;; shot e2e-ade-profiles;; key esc;; sleep 500;; expect-dialog closed;; action monitor;; sleep 600;; expect-dialog open;; key esc;; sleep 500;; expect-dialog closed;; wait-shown usage-meter;; click usage-meter;; sleep 400;; expect-shown monitor-popover;; shot e2e-ade-popover;; click usage-meter;; sleep 300;; expect-hidden monitor-popover;; quit"

# 4. Phone Pairing view, QR code rendering, and real Mobile Companion HTTP API handshake.
PAIR_INFO="$WORK/pair-info.json"
PAIR_CHECK="$WORK/pair-check.py"
cat > "$PAIR_CHECK" <<'PY'
import json, sys, time, urllib.request, urllib.error
info_path = sys.argv[1]
for _ in range(40):
    try:
        with open(info_path) as f:
            info = json.load(f)
        break
    except Exception:
        time.sleep(0.25)
else:
    sys.exit("pair info missing")
base = f"http://127.0.0.1:{info['port']}"
html = urllib.request.urlopen(base + "/").read().decode()
assert "TernaryCode Companion" in html, "companion HTML missing"
# Unauthorized state request must fail with 401.
try:
    urllib.request.urlopen(base + "/api/state")
    sys.exit("expected 401")
except urllib.error.HTTPError as e:
    assert e.code == 401, f"got {e.code}"
# Pair with real pairing token.
req = urllib.request.Request(
    base + "/api/pair",
    data=json.dumps({"token": info["token"], "device_name": "E2E iPhone"}).encode(),
    headers={"Content-Type": "application/json"},
)
paired = json.loads(urllib.request.urlopen(req).read().decode())
bearer = paired["device_token"]
assert bearer and paired["device_id"], "missing device_token"
# Authorized state request returns projects + profiles.
req = urllib.request.Request(base + "/api/state", headers={"Authorization": f"Bearer {bearer}"})
state = json.loads(urllib.request.urlopen(req).read().decode())
assert len(state["profiles"]) >= 2, f"expected profiles, got {state}"
assert any(p["name"] == "alpha" for p in state["projects"]), f"expected alpha project, got {state}"
open(info_path + ".ok", "w").write("PASS\n")
PY
run ade-pairing 60 "sleep 1000;; action add-project $P1;; sleep 1000;; action pairing;; sleep 500;; expect-view pairing;; action toggle-companion;; sleep 1000;; expect-shown pairing-qr;; expect-companion-url $PAIR_INFO;; spawn python3 $PAIR_CHECK $PAIR_INFO;; sleep 2500;; expect-file $PAIR_INFO.ok PASS;; shot e2e-ade-pairing;; action toggle-companion;; sleep 500;; quit"

echo "---- $PASS passed, $FAIL failed (logs: $WORK)"
[ "$FAIL" -eq 0 ]
