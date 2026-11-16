#!/usr/bin/env bash
# End-to-end checks of the Browser panel on every system (debug build): macOS, Linux (an X11
# session) and, with its twin scripts/e2e-web.ps1, Windows. Each scenario runs TernaryCode with a
# private data folder and a `TERNARYCODE_DEV_SCRIPT`. The steps act through the app (its actions,
# its widgets, an agent's browser tools run by a preset, real clicks in pages), so the same
# scenarios check each system's web view: WebKit, WebView2, WebKitGTK.
#
#   scripts/e2e-web.sh            # all scenarios
#   scripts/e2e-web.sh agent      # only scenarios whose name contains "agent"
#
# Linux: run it in the X11 session (DISPLAY set); `xdotool` (if installed) types keys into a page
# for the scenario of the app's keys pressed in pages.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
cargo build >/dev/null 2>&1 || { echo "build failed"; exit 1; }
if [ "$(uname)" = Linux ]; then
    cargo build -p tc-web >/dev/null 2>&1 || { echo "page helper build failed"; exit 1; }
fi
WORK=$(mktemp -d "${TMPDIR:-/tmp}/tc-e2e-web.XXXXXX")
BIN="$ROOT/target/debug/ternarycode"
HOOK="$ROOT/target/debug/ternarycode-hook"
ONLY="${1:-}"
PASS=0; FAIL=0

run() { # name, seconds, script
    local name="$1" secs="$2" script="$3"
    if [ -n "$ONLY" ] && [[ "$name" != *"$ONLY"* ]]; then return; fi
    local data="$WORK/data-$name"
    mkdir -p "$data"
    TERNARYCODE_DATA_DIR="$data" TERNARYCODE_SHOT_DIR="$WORK" TERNARYCODE_DEV_SCRIPT="$script" TERNARYCODE_DOWNLOAD_DIR="$WORK/downloads" \
        perl -e 'alarm shift; exec @ARGV' "$secs" "$BIN" > "$WORK/$name.log" 2>&1
    local p f done
    p=$(grep -c "DEVTEST PASS" "$WORK/$name.log"); f=$(grep -c "DEVTEST FAIL" "$WORK/$name.log"); done=$(grep -c "DEVTEST DONE" "$WORK/$name.log")
    PASS=$((PASS + p)); FAIL=$((FAIL + f))
    if [ "$f" -eq 0 ] && [ "$done" -gt 0 ]; then
        echo "ok    $name ($p checks)"
    else
        echo "FAIL  $name ($p passed, $f failed, done=$done)"; grep "DEVTEST FAIL" "$WORK/$name.log" | sed 's/^/        /'
        [ "$done" -eq 0 ] && FAIL=$((FAIL + 1))
    fi
}

# A preset whose command runs an agent's tool calls in its pane: `m NAME` pipes NAME.json into
# the MCP server (as an agent starts it) and writes NAME.out, then says ok-NAME.
preset() { # data dir name, preset id, calls…
    local name="$1" id="$2"; shift 2
    local script="$WORK/$id.sh"
    { printf 'm() { "%s" mcp claude < "%s/$1.json" > "%s/$1.out"; echo "ok-$1"; }\n' "$HOOK" "$MCP" "$MCP"; for c in "$@"; do printf '%s\n' "$c"; done; } > "$script"
    mkdir -p "$WORK/data-$name"
    printf '{"browser_agent_js":true,"presets":[{"id":"%s","name":"%s","command":"sh %s"}]}\n' "$id" "$id" "$script" > "$WORK/data-$name/settings.json"
}

P1="$WORK/alpha"; mkdir -p "$P1"; printf 'upload me\n' > "$P1/upload.txt"
SITE="$WORK/site"; mkdir -p "$SITE"; DL="$WORK/downloads"; mkdir -p "$DL"
MCP="$WORK/mcp"; mkdir -p "$MCP"
call() { printf '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"%s","arguments":%s}}\n' "$2" "$3" > "$MCP/$1.json"; }
WEBPORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server "$WEBPORT" --bind 127.0.0.1 --directory "$SITE" >/dev/null 2>&1 & WEB_PID=$!
trap 'kill $WEB_PID 2>/dev/null' EXIT
W="http://localhost:$WEBPORT"

# The pages.
printf '<!doctype html><title>TC Test Page</title><h1>Hello from the dev server</h1><a href="/second.html">Second page</a>\n' > "$SITE/index.html"
printf '<!doctype html><title>Second</title><h2>Second page</h2>\n' > "$SITE/second.html"
printf '<!doctype html><title>Agent Page</title><h1>Shop</h1><p id="count">Count: 0</p><script>let n = 0; function add() { n++; document.getElementById("count").textContent = "Count: " + n; console.log("clicked", n); }</script><button onclick="add()">Add one</button><input placeholder="Search"><a href="/second.html">Second page</a> <a href="https://example.com/">Elsewhere</a>\n' > "$SITE/agent.html"
printf 'hello download\n' > "$SITE/file.txt"
printf '<!doctype html><title>Dialogs</title><button id="rm" onclick="document.title = confirm(&quot;Remove Wool Beanie?&quot;) ? &quot;Removed&quot; : &quot;Kept&quot;">Remove</button><button id="hi" onclick="alert(&quot;Hello there&quot;); document.title = &quot;Alerted&quot;">Hello</button><button id="nm" onclick="document.title = &quot;Name: &quot; + prompt(&quot;Your name?&quot;, &quot;Dina&quot;)">Name</button><a id="dl" href="/file.txt" download>Get the file</a>\n' > "$SITE/dialogs.html"
printf '<!doctype html><title>Find Page</title><p>Linen shirt</p><p>Wool beanie</p><p>linen tote and LINEN scarf</p>\n' > "$SITE/find.html"
printf '<!doctype html><title>Pay Frame</title><body style="margin:0"><input id="card" style="position:absolute;left:10px;top:10px;width:200px;height:30px"><button style="position:absolute;left:10px;top:60px;width:100px;height:30px" onclick="parent.postMessage(&quot;paid:&quot; + document.getElementById(&quot;card&quot;).value, &quot;*&quot;)">Pay</button></body>\n' > "$SITE/frame.html"
printf '<!doctype html><title>Human Page</title><body style="margin:0;font:14px sans-serif"><form onsubmit="event.preventDefault(); document.title = &quot;Submitted: &quot; + this.q.value"><input name="q" aria-label="Search field" style="position:absolute;left:10px;top:10px;width:150px"><button style="position:absolute;left:170px;top:10px">Search</button></form><div role="button" style="position:absolute;left:10px;top:50px;width:100px;height:30px;background:#ddd" onmouseenter="document.title = &quot;Hovered&quot;">Hover me</div><div id="a" role="button" style="position:absolute;left:10px;top:90px;width:80px;height:30px;background:#cde">Drag A</div><div id="b" role="button" style="position:absolute;left:200px;top:90px;width:80px;height:30px;background:#edc">Target B</div><div role="button" draggable="true" style="position:absolute;left:10px;top:130px;width:80px;height:30px;background:#dec" ondragstart="event.dataTransfer.setData(&quot;text&quot;, &quot;c&quot;)">Card C</div><div role="button" style="position:absolute;left:200px;top:130px;width:80px;height:30px;background:#ced" ondragover="event.preventDefault()" ondrop="document.title = &quot;HTML5 drop &quot; + event.dataTransfer.getData(&quot;text&quot;)">Drop D</div><input type="file" aria-label="Receipt" style="position:absolute;left:10px;top:170px" onchange="document.title = &quot;File: &quot; + this.files[0].name"><iframe title="Pay" src="http://127.0.0.1:%s/frame.html" style="position:absolute;left:20px;top:220px;width:320px;height:120px;border:0"></iframe><script>let down = false; document.getElementById("a").addEventListener("pointerdown", () => down = true); document.addEventListener("pointerup", (e) => { const t = document.elementFromPoint(e.clientX, e.clientY); if (down && t && t.id === "b") document.title = "Dropped on B"; down = false; }); addEventListener("message", (e) => { if (String(e.data).startsWith("paid:")) document.title = "Paid: " + String(e.data).slice(5); });</script></body>\n' "$WEBPORT" > "$SITE/human.html"
printf '<!doctype html><title>Popup Opener</title><button id="signin" style="font:30px sans-serif;margin:40px;padding:20px">Sign in</button><script>document.getElementById("signin").onclick = function () { window.open("/signin.html", "signin", "width=480,height=560"); }; window.addEventListener("message", function (e) { if (e.origin === location.origin) document.title = "Signed in: " + e.data; });</script>\n' > "$SITE/popup.html"
printf '<!doctype html><title>Sign In Page</title><h1>Sign in</h1><script>setTimeout(function () { window.opener.postMessage("tc-ok", "*"); window.close(); }, 1500);</script>\n' > "$SITE/signin.html"
printf '<!doctype html><title>New Window Page</title><a id="nw" href="/second.html" target="_blank" style="display:block;font:40px sans-serif;padding:30px 20px;height:200px">New window</a>\n' > "$SITE/newwin.html"
printf '<!doctype html><title>Keys Page</title><input id="k" autofocus style="font:30px sans-serif" oninput="document.title = &quot;Keys: &quot; + this.value">\n' > "$SITE/keys.html"

# A page in the panel: placed where its pane is, hidden under the palette (its picture shows),
# back after; another address typed into its address field; a second page in a new tab; the
# first one again by its tab.
run web-basic 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/;; wait-browser TC Test Page;; wait-pages 3*;; wait-panel shown;; sleep 800;; expect-web-placed;; web-shot web-basic-page;; shot web-basic;; action palette;; sleep 600;; expect-web-hidden;; key esc;; sleep 600;; expect-web-placed;; action browser-address;; sleep 300;; keys $W/second.html\\r;; wait-browser Second;; action new-browser-tab;; wait-pages 3,4*;; action browser-address;; sleep 300;; keys $W/\\r;; wait-browser TC Test Page;; click page-tab:3;; wait-pages 3*,4;; wait-browser Second;; sleep 500;; expect-web-placed;; quit"

# An agent's browser tools: open, read, click, type, the console, its own script, a picture,
# a link, back, scroll; a link to another site asks the user first (Block).
call nav browser_navigate "{\"url\":\"localhost:$WEBPORT/agent.html\"}"
call add browser_click '{"ref":"e1","element":"Add one button"}'
call typ browser_type '{"ref":"e2","element":"Search field","text":"linen"}'
call con browser_console_messages '{}'
call js browser_evaluate '{"function":"() => document.title + \" / \" + n"}'
call pic browser_take_screenshot '{}'
call sec browser_click '{"ref":"e3","element":"Second page link"}'
call bak browser_navigate_back '{}'
call scr browser_scroll '{"direction":"down"}'
call away browser_click '{"ref":"e4","element":"Elsewhere link"}'
preset web-agent agent 'm nav; m add; m typ; m con; m js; m pic; m sec; m bak; m scr; m away'
run web-agent 120 "sleep 1000;; action add-project $P1;; sleep 1500;; action run agent;; wait-screen ok-nav;; wait-pages 3*;; expect-file $MCP/nav.out Page: Agent Page;; expect-file $MCP/nav.out [ref=e1];; wait-screen ok-add;; expect-file $MCP/add.out Count: 1;; wait-screen ok-typ;; expect-file $MCP/typ.out linen;; wait-screen ok-con;; expect-file $MCP/con.out [log] clicked 1;; wait-screen ok-js;; expect-file $MCP/js.out Agent Page / 1;; wait-screen ok-pic;; expect-file $MCP/pic.out image/png;; wait-screen ok-sec;; expect-file $MCP/sec.out Page: Second;; wait-screen ok-bak;; expect-file $MCP/bak.out Page: Agent Page;; wait-screen ok-scr;; expect-file $MCP/scr.out Scrolled down.;; wait-shown browser-ask:3;; shot web-agent-ask;; click browser-block:3;; wait-screen ok-away;; expect-file $MCP/away.out Blocked by the user: example.com;; quit"

# An agent as a person: keys typed and submitted, hover, a pointer drag and the page's own drag
# and drop, a file chosen for a file field (one outside the project refused), clicks and keys
# at points of a frame of another site.
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
preset web-agent-human hum 'm hnav; m hq; m htype; m hhover; m hdrag; m hdnd; m hup; m hbad; m hshot; m hxy1; m htype2; m hxy2'
run web-agent-human 120 "sleep 1000;; action add-project $P1;; sleep 1500;; action run hum;; wait-screen ok-htype;; wait-screen ok-hdrag;; wait-screen ok-hbad;; wait-screen ok-hxy1;; wait-screen ok-hxy2;; expect-file $MCP/hnav.out [ref=e8];; expect-file $MCP/htype.out Page: Submitted: hello;; expect-file $MCP/hhover.out Page: Hovered;; expect-file $MCP/hdrag.out Page: Dropped on B;; expect-file $MCP/hdnd.out Page: HTML5 drop c;; expect-file $MCP/hup.out Page: File: upload.txt;; expect-file $MCP/hbad.out outside the project;; expect-file $MCP/hshot.out CSS pixels;; expect-file $MCP/hxy2.out Page: Paid: 4242;; web-shot web-agent-human;; quit"

# A page's dialogs show as a bar above it and the page goes on with the answer; a download
# lands in the downloads folder and is listed.
run web-dialogs 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/dialogs.html;; wait-browser Dialogs;; wait-pages 3*;; sleep 800;; web-click document.getElementById('rm');; wait-dialog Remove Wool Beanie;; expect-shown browser-dialog:3;; shot web-dialog;; click browser-dialog-cancel:3;; wait-browser Kept;; expect-hidden browser-dialog:3;; web-click document.getElementById('rm');; wait-dialog Remove Wool Beanie;; click browser-dialog-ok:3;; wait-browser Removed;; web-click document.getElementById('hi');; wait-dialog Hello there;; click browser-dialog-ok:3;; wait-browser Alerted;; web-click document.getElementById('nm');; wait-dialog Your name;; click browser-dialog-ok:3;; wait-browser Name: Dina;; web-click document.getElementById('dl');; wait-download file.txt;; expect-file $DL/file.txt hello download;; expect-shown browser-downloads:3;; quit"

# An agent told of a dialog its click opened answers it; it hears where a download went.
call dnav browser_navigate "{\"url\":\"localhost:$WEBPORT/dialogs.html\"}"
call drm browser_click '{"ref":"e1","element":"Remove button"}'
call dno browser_handle_dialog '{"accept":false}'
call dnm browser_click '{"ref":"e3","element":"Name button"}'
call dyes browser_handle_dialog '{"accept":true,"promptText":"Ada"}'
call ddl browser_click '{"ref":"e4","element":"Get the file link"}'
preset web-agent-dialogs dlg 'm dnav; m drm; m dno; m dnm; m dyes; m ddl'
rm -f "$DL/file.txt"
run web-agent-dialogs 90 "sleep 1000;; action add-project $P1;; sleep 1500;; action run dlg;; wait-screen ok-ddl;; expect-file $MCP/drm.out A dialog is open in the page (confirm;; expect-file $MCP/dno.out Dismissed the dialog;; expect-file $MCP/dno.out Page: Kept;; expect-file $MCP/dnm.out A dialog is open in the page (prompt;; expect-file $MCP/dyes.out Page: Name: Ada;; expect-file $MCP/ddl.out Downloaded file.txt to $DL/file.txt;; quit"

# Find in page (counted, round), zoom (the toolbar's chip shows it; a click resets it), the
# Web Inspector.
run web-tools 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/find.html;; wait-browser Find Page;; wait-pages 3*;; sleep 800;; page-find linen;; wait-page-find 1/3;; expect-shown browser-find:3;; page-find-next;; wait-page-find 2/3;; page-find-next;; wait-page-find 3/3;; page-find-next;; wait-page-find 1/3;; page-find-prev;; wait-page-find 3/3;; shot web-find;; page-find zzz;; wait-page-find 0/0;; page-zoom 1;; sleep 400;; expect-page-zoom 1.1;; expect-shown browser-zoom:3;; click browser-zoom:3;; sleep 400;; expect-page-zoom 1;; page-inspect;; wait-inspector;; quit"

# A sign-in pop-up (window.open with a size, after a click in the page): it opens tied to the
# page, reports back through window.opener and closes itself; the page goes on signed in, with
# no new page in the panel. A link to a new window opens a new page in the panel.
run web-popup 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/popup.html;; wait-browser Popup Opener;; wait-pages 3*;; sleep 1000;; web-click document.getElementById('signin');; wait-browser Signed in: tc-ok;; wait-pages 3*;; action open-web $W/newwin.html;; wait-browser New Window Page;; sleep 800;; web-click document.getElementById('nw');; wait-pages 3,4*;; wait-browser Second;; quit"

# An agent's click that opens a sign-in pop-up: the agent hears of it and its next steps act in
# it; its click there signs the page in and the pop-up closes; browser_navigate closes an open
# pop-up. A pop-up to a site the agent may not open is asked about first (Block: not opened).
printf '<!doctype html><title>Agent Popup Opener</title><button onclick="window.open(&quot;/agentsignin.html&quot;, &quot;s&quot;, &quot;width=480,height=560&quot;)">Sign in</button><button onclick="window.open(&quot;http://tc-popup.invalid/x&quot;, &quot;w&quot;, &quot;width=400,height=400&quot;)">Sign in elsewhere</button><script>window.addEventListener("message", function (e) { if (e.origin === location.origin) document.title = "Signed in: " + e.data; });</script>\n' > "$SITE/agentpop.html"
printf '<!doctype html><title>Agent Sign In</title><button onclick="window.opener.postMessage(&quot;agent-ok&quot;, &quot;*&quot;); window.close();">Continue</button>\n' > "$SITE/agentsignin.html"
call pnav browser_navigate "{\"url\":\"localhost:$WEBPORT/agentpop.html\"}"
call pclick browser_click '{"ref":"e1","element":"Sign in button"}'
call pcont browser_click '{"ref":"e1","element":"Continue button"}'
call pclick2 browser_click '{"ref":"e1","element":"Sign in button"}'
call pnav2 browser_navigate "{\"url\":\"localhost:$WEBPORT/agentpop.html\"}"
call pelse browser_click '{"ref":"e2","element":"Sign in elsewhere button"}'
preset web-agent-popup pop 'm pnav; m pclick; sleep 3; m pcont; m pclick2; sleep 2; m pnav2; m pelse'
run web-agent-popup 90 "sleep 1000;; action add-project $P1;; sleep 1500;; action run pop;; wait-screen ok-pnav;; expect-file $MCP/pnav.out Page: Agent Popup Opener;; wait-screen ok-pclick;; expect-file $MCP/pclick.out A pop-up window opened;; expect-file $MCP/pclick.out Page: Agent Sign In (a pop-up window);; wait-screen ok-pcont;; expect-file $MCP/pcont.out The pop-up window closed;; expect-file $MCP/pcont.out Page: Signed in: agent-ok;; wait-screen ok-pclick2;; wait-screen ok-pnav2;; expect-file $MCP/pnav2.out The pop-up window closed;; expect-file $MCP/pnav2.out Page: Agent Popup Opener;; wait-shown browser-ask:3;; click browser-block:3;; wait-screen ok-pelse;; expect-file $MCP/pelse.out Blocked by the user: tc-popup.invalid;; quit"

# The app's keys pressed while a page has the keyboard (Windows, Linux: the page's window gets
# them): Ctrl+F opens the page's find bar, Ctrl+L its address field; other keys stay the page's.
if [ "$(uname)" = Linux ] && command -v xdotool >/dev/null; then
    # A real click on the page (the window's page area), then real keys.
    printf 'W=$(xdotool search --onlyvisible --name "^TernaryCode$" | head -1); eval $(xdotool getwindowgeometry --shell "$W"); xdotool mousemove $((X + WIDTH * 3 / 4)) $((Y + 130)) click 1; sleep 1; xdotool type --delay 80 abc; sleep 1; xdotool key ctrl+f\n' > "$WORK/xkeys.sh"
    run web-keys 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/keys.html;; wait-browser Keys Page;; wait-pages 3*;; sleep 1500;; spawn sh $WORK/xkeys.sh;; wait-page-focus;; wait-browser Keys: abc;; wait-shown browser-find:3;; quit"
fi

echo "== $PASS checks passed, $FAIL failed (logs in $WORK)"
[ "$FAIL" -eq 0 ]
