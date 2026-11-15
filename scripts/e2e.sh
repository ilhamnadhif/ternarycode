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
    TERNARYCODE_DATA_DIR="$data" TERNARYCODE_SHOT_DIR="$WORK" TERNARYCODE_DEV_SCRIPT="$script" \
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
# a local web server); its chip opens the page beside the terminal, where the web view sits
# exactly on the pane; an address typed goes there; ⇧⌘B opens the project's server in a tab of
# its own. After a restart both pages come back where they were.
if [ "$(uname)" = Darwin ] && { [ -z "$ONLY" ] || [[ "browser-1 browser-2 browser-agent browser-more browser-grid browser-button" == *"$ONLY"* ]]; }; then
    SITE="$WORK/site"; mkdir -p "$SITE"
    printf '<!doctype html><title>TC Test Page</title><h1>Hello from the dev server</h1><a href="/second.html">Second page</a>\n' > "$SITE/index.html"
    printf '<!doctype html><title>Second</title><h2>Second page</h2>\n' > "$SITE/second.html"
    WEBPORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    python3 -m http.server "$WEBPORT" --bind 127.0.0.1 --directory "$SITE" >/dev/null 2>&1 & WEB_PID=$!; disown
    printf '#!/bin/sh\nprintf "  Local:   http://localhost:%%s/\\n" "$1"\nexec sleep 600\n' > "$WORK/bin/announce"; chmod +x "$WORK/bin/announce"
    run browser-1 60 "sleep 1000;; action add-project $P1;; sleep 1200;; keys $WORK/bin/announce $WEBPORT\\r;; wait-shown server-chip;; click server-chip;; wait-browser TC Test Page;; wait-tabs 0:2,3;; sleep 500;; expect-web-placed;; web-shot e2e-browser-page;; shot e2e-browser;; click browser-address:3;; sleep 200;; key cmd+a;; keys localhost:$WEBPORT/second.html\\r;; wait-browser Second;; action new-browser-tab;; wait-tabs 0:2,3 1:4;; wait-browser TC Test Page;; sleep 500;; expect-web-placed;; action palette;; sleep 500;; expect-web-hidden;; key esc;; sleep 500;; expect-web-placed;; click browser-more:4;; sleep 400;; expect-web-hidden;; shot e2e-browser-menu;; key esc;; sleep 500;; expect-web-placed;; quit"
    mkdir -p "$WORK/data-browser-2"; cp "$WORK/data-browser-1/workspace.json" "$WORK/data-browser-2/" 2>/dev/null
    run browser-2 50 "sleep 2000;; expect-project alpha;; wait-tabs 0:2,3 1:4;; wait-browser TC Test Page;; sleep 500;; expect-web-placed;; action select-tab 0;; wait-browser Second;; sleep 500;; expect-web-placed;; quit"

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
    mkdir -p "$WORK/data-browser-agent"; printf '{"browser_agent_js":true}\n' > "$WORK/data-browser-agent/settings.json"
    HOOK="$ROOT/target/debug/ternarycode-hook"
    run browser-agent 120 "sleep 1000;; action add-project $P1;; sleep 1500;; keys m() { $HOOK mcp claude < $MCP/\$1.json > $MCP/\$1.out; echo ok-\$1; }; clear\\r;; sleep 300;; keys m nav\\r;; wait-screen ok-nav;; wait-tabs 0:2,3;; expect-file $MCP/nav.out Page: Agent Page;; expect-file $MCP/nav.out [ref=e1];; wait-browser Agent Page;; keys m add\\r;; wait-screen ok-add;; expect-file $MCP/add.out Clicked Add one button.;; expect-file $MCP/add.out Count: 1;; keys m typ\\r;; wait-screen ok-typ;; expect-file $MCP/typ.out linen;; keys m con\\r;; wait-screen ok-con;; expect-file $MCP/con.out [log] clicked 1;; keys m js\\r;; wait-screen ok-js;; expect-file $MCP/js.out Agent Page / 1;; keys m pic\\r;; wait-screen ok-pic;; expect-file $MCP/pic.out image/png;; keys m sec\\r;; wait-screen ok-sec;; expect-file $MCP/sec.out Page: Second;; keys m bak\\r;; wait-screen ok-bak;; expect-file $MCP/bak.out Page: Agent Page;; keys m scr\\r;; wait-screen ok-scr;; expect-file $MCP/scr.out Scrolled down.;; hook 2 claude SessionStart;; keys m away &\\r;; wait-shown browser-ask:3;; wait-status needs-input;; expect-web-placed;; shot e2e-browser-ask;; click browser-block:3;; wait-screen ok-away;; wait-status running;; expect-file $MCP/away.out Blocked by the user: example.com;; wait-browser Agent Page;; keys m inv &\\r;; wait-shown browser-ask:3;; click browser-always:3;; wait-screen ok-inv;; expect-file $MCP/inv.out Opened http://tc-e2e.invalid/;; keys m inv2\\r;; wait-screen ok-inv2;; expect-file $MCP/inv2.out Opened http://tc-e2e.invalid/two;; expect-hidden browser-ask:3;; keys clear; m nav\\r;; wait-screen ok-nav;; keys m hold &\\r;; sleep 1000;; wait-shown browser-take-over:3;; shot e2e-browser-agent;; click browser-take-over:3;; sleep 400;; expect-shown browser-take-over:3;; focus-pane 0;; wait-screen ok-hold;; expect-file $MCP/hold.out took the browser over from Claude Code;; keys m snap\\r;; wait-screen ok-snap;; expect-file $MCP/snap.out has the browser now;; click browser-take-over:3;; sleep 300;; keys clear; m snap\\r;; wait-screen ok-snap;; expect-file $MCP/snap.out Page: Agent Page;; quit"
    expect_saved() { grep -q "$2" "$WORK/data-$1/workspace.json" && echo "ok    $1 saved $2" || { echo "FAIL  $1 did not save $2"; FAIL=$((FAIL + 1)); }; }
    [ -z "$ONLY" ] || [[ "browser-agent" == *"$ONLY"* ]] && expect_saved browser-agent tc-e2e.invalid

    # More of the browser: $BROWSER (Python's webbrowser) opens the page beside the terminal; its
    # console's error and warning show as a chip, then in the drawer; under the palette the page
    # is a picture; `open https://…` and `ternarycode browse` go to the same browser; a link to a
    # new window, clicked, opens a tab; a page whose server stops says so, and comes back by
    # itself when it answers again; an agent's own JavaScript is refused unless allowed.
    printf '<!doctype html><title>Console Page</title><h1>Console</h1><script>console.log("hello"); console.warn("slow image"); setTimeout(function () { null.qty; }, 50);</script>\n' > "$SITE/console.html"
    printf '<!doctype html><title>New Window Page</title><a href="/second.html" target="_blank" style="display:block;font:40px sans-serif;padding:30px 20px;height:200px">New window</a>\n' > "$SITE/newwin.html"
    DOWNPORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    call evl browser_evaluate '{"function":"() => 1"}'
    run browser-more 90 "sleep 1000;; action add-project $P1;; sleep 1500;; keys python3 -c 'import webbrowser; webbrowser.open(\"http://localhost:$WEBPORT/console.html\")'\\r;; wait-browser Console Page;; wait-tabs 0:2,3;; sleep 1000;; expect-shown browser-console:3;; click browser-console:3;; sleep 400;; expect-shown browser-console-drawer:3;; sleep 300;; expect-web-placed;; shot e2e-browser-console;; action palette;; sleep 800;; expect-web-hidden;; shot e2e-browser-still;; key esc;; sleep 600;; expect-web-placed;; focus-pane 0;; keys clear; open http://localhost:$WEBPORT/second.html\\r;; wait-browser Second;; keys \"\$(dirname \"\$TERNARYCODE_HOOK\")/ternarycode\" browse http://localhost:$WEBPORT/newwin.html\\r;; wait-browser New Window Page;; sleep 800;; mouse down browser:3@0.3,0.08;; mouse up browser:3@0.3,0.08;; wait-tabs 0:2,3 1:4;; wait-browser Second;; action select-tab 0;; sleep 300;; focus-pane 0;; keys \"\$(dirname \"\$TERNARYCODE_HOOK\")/ternarycode\" browse http://localhost:$DOWNPORT/\\r;; sleep 2500;; expect-shown browser-down:3;; shot e2e-browser-down;; keys python3 -m http.server $DOWNPORT --bind 127.0.0.1 &\\r;; sleep 4000;; expect-hidden browser-down:3;; wait-browser Directory listing;; keys kill %1; $HOOK mcp claude < $MCP/evl.json > $MCP/evl.out; echo ok-\$((1+1))-evl\\r;; wait-screen ok-2-evl;; expect-file $MCP/evl.out JavaScript in pages is off;; action settings-browser;; sleep 600;; shot e2e-settings-browser;; key esc;; quit"
    # The Grid: an agent's browser in a tab of its own shows as a picture in the corner of the
    # agent's tile; a click on it shows the page in the tile, where its web view is laid.
    mkdir -p "$WORK/data-browser-grid"; printf '{"browser_agent_beside":false}\n' > "$WORK/data-browser-grid/settings.json"
    call hold2 browser_wait_for '{"text":"never shows","time":15}'
    run browser-grid 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys $HOOK mcp claude < $MCP/nav.json > $MCP/nav2.out; $HOOK mcp claude < $MCP/hold2.json > $MCP/hold2.out &\\r;; wait-tabs 0:2 1:3;; action grid;; sleep 2000;; expect-shown tile-pip:1;; shot e2e-browser-pip;; click tile-pip:1;; sleep 1200;; expect-shown tile-browser:3;; expect-web-placed;; quit"
    # The Browser button in the top bar: with a dev server announced, it opens the page beside the
    # terminal; again, its address field takes what is typed; from another tab it brings the
    # browser's tab back; ⌘R reloads the page; ⌘L in a project with no browser opens one, its
    # address field taking the keyboard.
    run browser-button 60 "sleep 1000;; action add-project $P1;; sleep 1500;; expect-shown browser-button;; keys $WORK/bin/announce $WEBPORT\\r;; wait-shown server-chip;; click browser-button;; wait-tabs 0:2,3;; wait-browser TC Test Page;; sleep 500;; expect-web-placed;; shot e2e-browser-button;; click browser-button;; sleep 300;; keys localhost:$WEBPORT/second.html\\r;; wait-browser Second;; action browser-reload;; sleep 800;; wait-browser Second;; action new-tab;; sleep 800;; wait-tabs 0:2,3 1:4;; click browser-button;; sleep 500;; expect-tab 0;; wait-browser Second;; action add-project $P2;; sleep 1500;; action browser-address;; sleep 500;; keys localhost:$WEBPORT\\r;; wait-browser TC Test Page;; quit"
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
run sidebar-active 60 "sleep 1000;; action add-project $P1;; sleep 1000;; action add-project $P2;; sleep 1200;; mouse move pane:4@0.5,0.5;; sleep 300;; expect-hidden sidebar-group:active;; expect-above project:alpha project:beta;; keys sleep 5\\r;; sleep 3000;; expect-shown sidebar-group:active;; expect-shown sidebar-group:idle;; expect-above project:beta project:alpha;; action select 0;; sleep 400;; expect-project beta;; click sidebar-group:idle;; sleep 300;; expect-hidden project:alpha;; click sidebar-group:idle;; sleep 300;; expect-shown project:alpha;; shot e2e-sidebar-active;; mouse move project:alpha@0.5,0.5;; sleep 3500;; expect-shown sidebar-group:active;; mouse move pane:4@0.5,0.5;; sleep 1200;; expect-hidden sidebar-group:active;; expect-above project:alpha project:beta;; quit"
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

run grid 60 "sleep 1000;; action add-project $P1;; sleep 900;; action split-right;; sleep 800;; keys echo alpha-left\\r;; action add-project $P2;; sleep 900;; action split-down;; sleep 800;; action add-project $R;; sleep 1200;; action grid;; sleep 1200;; expect-fill 2;; expect-fill 3;; expect-fill 5;; expect-fill 6;; expect-fill 8;; shot e2e-grid;; action focus;; sleep 1200;; expect-fill 8;; action select 0;; sleep 1200;; expect-fill 2;; expect-fill 3;; wait-screen alpha-left;; click view:grid;; sleep 1200;; expect-shown tile:beta;; shot e2e-all-projects;; click view:focus;; sleep 1200;; expect-hidden tile:beta;; click view:grid;; sleep 1200;; expect-shown tile:beta;; click project:beta;; sleep 1200;; expect-project beta;; expect-hidden tile:alpha;; shot e2e-focus-beta;; quit"

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
run explorer 60 "sleep 1000;; action add-project $T;; sleep 1500;; action files;; sleep 800;; wait-tree edit.txt;; click tree:edit.txt;; wait-open edit.txt;; expect-rect editor-text;; write-file $T/edit.txt line 1 changed by agent\\n;; wait-git edit.txt M;; write-file $T/added.txt new\\n;; wait-tree added.txt;; wait-git added.txt U;; click editor-close;; sleep 400;; click toggle-editor;; wait-open edit.txt;; shot e2e-explorer;; quit"


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
run explorer-filter-esc 40 "sleep 1000;; action add-project $T;; sleep 1500;; action files;; sleep 800;; wait-tree edit.txt;; click files-filter;; sleep 300;; keys zzz;; sleep 500;; expect-hidden tree:edit.txt;; key esc;; sleep 500;; expect-shown tree:edit.txt;; quit"
run files-actions 50 "sleep 1000;; action add-project $F;; sleep 1500;; action files;; sleep 800;; wait-tree a.txt;; sleep 300;; click files-button:0;; sleep 400;; keys hello.txt\\r;; wait-open hello.txt;; wait-tree hello.txt;; action rename hello.txt;; sleep 400;; keys renamed\\r;; wait-tree renamed.txt;; wait-open renamed.txt;; click files-button:1;; sleep 400;; keys docs/api\\r;; wait-tree docs;; shot e2e-files;; quit"
run ctrl-d 40 "sleep 1000;; action add-project $P1;; sleep 1200;; action split-right;; sleep 2600;; wait-tabs 0:2,3;; key ctrl+d;; wait-tabs 0:2;; sleep 300;; key ctrl+d;; wait-tabs 0:4;; sleep 600;; key ctrl+d;; wait-tabs 0:5;; quit"
# Review opened from "N changed" shows those N, since the last commit, in the Code drawer (not a
# page of its own). A new folder made before the agent started: the agent's own run has nothing,
# so Review from the agent falls back to the last commit; asked for the run, it says why it is
# empty and offers the changes since the last commit. Full screen and back with Esc.
RS="$WORK/scope"; mkdir -p "$RS" && (cd "$RS" && git init -q -b main && printf 'one\n' > a.txt && git add -A && git -c user.name=t -c user.email=t@t commit -qm init && mkdir -p notes && printf 'x\n' > notes/n1.md && printf 'y\n' > notes/n2.md)
run review-scope 70 "sleep 1000;; action add-project $RS;; sleep 1500;; hook 2 claude SessionStart;; sleep 300;; wait-snapshot;; action review;; wait-review Added:notes/n1.md,Added:notes/n2.md;; expect-review Head drawer;; expect-shown review-dir:notes;; action review-agent;; wait-review Added:notes/n1.md,Added:notes/n2.md;; expect-review Head drawer;; click review-base:Run;; sleep 800;; wait-review ;; expect-review Run drawer;; expect-shown review-show-head;; shot e2e-review-scope-empty;; click review-show-head;; wait-review Added:notes/n1.md,Added:notes/n2.md;; expect-review Head drawer;; click review-expand;; sleep 400;; expect-review Head full;; key esc;; sleep 400;; expect-review Head drawer;; shot e2e-review-drawer;; quit"

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

# A Grid tile of a project with two tabs (alpha: the agent's tab 0, a shell in tab 1, the one
# last open; the keyboard is in beta's tile): it shows the tab last open while nothing happens;
# the agent asking brings its tab; a tab picked from the strip holds until something new waits;
# the tile typed into never changes by itself; ↗ opens Focus on the tab shown.
run grid-tabs 60 "sleep 1000;; action add-project $P1;; sleep 1200;; action new-tab;; sleep 800;; wait-tabs 0:2 1:3;; action add-project $P2;; sleep 1200;; action grid;; sleep 1200;; expect-tile-tab alpha 1;; hook 2 claude PermissionRequest;; sleep 800;; expect-tile-tab alpha 0;; expect-shown tile-tab:alpha:1;; shot e2e-grid-tabs;; click tile-tab:alpha:1;; sleep 600;; expect-tile-tab alpha 1;; sleep 1200;; expect-tile-tab alpha 1;; hook 2 claude Stop;; sleep 800;; expect-tile-tab alpha 0;; click tile-tab:alpha:1;; sleep 600;; expect-tile-tab alpha 1;; click tile-pane:3;; sleep 500;; hook 2 claude PermissionRequest;; sleep 800;; expect-tile-tab alpha 1;; click tile-open:alpha;; sleep 800;; expect-project alpha;; expect-tab 1;; quit"

echo "---- $PASS passed, $FAIL failed (logs: $WORK)"
[ "$FAIL" -eq 0 ]
