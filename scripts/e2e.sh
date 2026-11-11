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

run smoke 40 "sleep 1200;; action add-project $P1;; sleep 1500;; keys echo hello-tc\\r;; wait-screen hello-tc;; wait-status idle;; expect-project alpha;; hook-latency claude PermissionRequest needs-input;; hook-latency claude Stop idle|done;; quit"

run jump 40 "sleep 1000;; action add-project $P2;; sleep 1000;; action add-project $P1;; sleep 1200;; hook 2 claude Notification permission_prompt;; sleep 400;; wait-queue 1;; action next-attention;; sleep 400;; expect-project beta;; wait-queue 0;; action back-attention;; sleep 400;; expect-project alpha;; action select 1;; sleep 400;; expect-project beta;; quit"

run foreground 40 "sleep 1000;; action add-project $P1;; sleep 1200;; action split-right;; sleep 1000;; focus-pane 0;; sleep 500;; keys sleep 30\\r;; wait-status running;; key ctrl+c;; sleep 500;; wait-status idle;; quit"

run hidden 40 "sleep 1000;; action add-project $P1;; sleep 1000;; action add-project $P2;; sleep 1000;; close-window;; sleep 1500;; expect-alive 2;; hook 2 claude Notification permission_prompt;; sleep 800;; wait-queue 1;; quit"

run quit-restore-1 40 "sleep 1000;; action add-project $P1;; sleep 1200;; hook 2 claude SessionStart;; sleep 500;; keys sleep 300\\r;; sleep 1200;; action quit;; sleep 500;; expect-dialog open;; click dialog-ok;; sleep 3000" no-done
mkdir -p "$WORK/data-quit-restore-2"; cp "$WORK/data-quit-restore-1/workspace.json" "$WORK/data-quit-restore-2/" 2>/dev/null
run quit-restore-2 40 "sleep 2000;; expect-project alpha;; expect-rect resume:2;; action grid;; action grid-active;; sleep 800;; expect-shown tile:alpha;; expect-rect tile-resume:2;; action grid-all;; quit"

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
run doc-find 60 "sleep 1000;; action add-project $DOCS;; sleep 1200;; open-file $DOCS/report.pdf;; wait-doc kind=pdf state=ready pages=2;; click doc-body;; sleep 200;; action find;; sleep 300;; keys quarterly;; wait-doc matches=2 current=1;; key enter;; wait-doc current=2;; shot e2e-doc-find-pdf;; key esc;; open-file $DOCS/data.csv;; wait-doc kind=csv state=ready;; click doc-body;; action find;; sleep 300;; keys beta;; wait-doc matches=1 cell=2,0;; key esc;; open-file $DOCS/notes.docx;; wait-doc kind=word state=ready;; click doc-body;; action find;; sleep 300;; keys docs;; wait-doc matches=1 current=1;; shot e2e-doc-find-word;; quit"

# The sidebar keeps the width it is dragged to (it used to shrink back to its narrowest).
run sidebar-resize 40 "sleep 1000;; action add-project $P1;; sleep 1200;; expect-width sidebar 230 250;; mouse down sidebar@1.0,0.5;; sleep 100;; mouse drag sidebar@1.4,0.5;; sleep 150;; mouse up sidebar@1.0,0.5;; sleep 1500;; expect-width sidebar 320 345;; sleep 1500;; expect-width sidebar 320 345;; quit"
run grid-fit 50 "sleep 1000;; action add-project $P1;; sleep 1200;; action split-right;; sleep 1500;; wait-tabs 0:2,3;; action grid;; sleep 1000;; expect-fill 2;; expect-fill 3;; mouse down tile:alpha@0.75,0.5;; mouse up tile:alpha@0.75,0.5;; sleep 300;; key ctrl+d;; wait-tabs 0:2;; sleep 800;; expect-fill 2;; shot e2e-grid-fit;; action focus;; sleep 1000;; expect-fill 2;; quit"

# A zsh prompt survives resizes whole (v1.2): split, close the split, open and close the editor;
# the screen keeps one prompt per command and no piece of an old one (shell integration marks
# the prompt, the resize clears it before the reflow, zsh redraws it).
run prompt-resize 60 "sleep 1000;; action add-project $P1;; sleep 1500;; keys echo hi\\r;; wait-screen hi;; sleep 600;; action split-right;; sleep 1500;; action close-pane;; sleep 1500;; action editor;; sleep 1500;; action editor;; sleep 1500;; expect-screen-count alpha % 2;; expect-no-line %;; shot e2e-prompt-resize;; quit"

# The editor's language features (v1.3), with a language server answering from plain text:
# go to definition (another file) and back, references, diagnostics, completion, rename across
# files (on disk) and its undo, a code action, formatting.
L="$WORK/lsp"; mkdir -p "$L/src"
printf 'fn main() {\n    let x = helper(1);\n    println!("{x}");\n}\n' > "$L/src/main.rs"
printf 'pub fn helper(a: i32) -> i32 {\n    a + 1\n}\n\n// BUG here\n' > "$L/src/util.rs"
run lsp 90 "sleep 1000;; action add-project $L;; sleep 1200;; open-file $L/src/main.rs;; wait-lsp ready;; goto $L/src/main.rs 1 14;; sleep 400;; action ide def;; wait-active util.rs;; wait-caret 1;; wait-diag 1 0;; action ide back;; wait-active main.rs;; wait-caret 2;; action ide refs;; wait-locations 2;; key esc;; sleep 300;; action ide rename;; wait-rename;; sleep 300;; keys helper2\\r;; sleep 1200;; expect-file $L/src/util.rs fn helper2(;; expect-file $L/src/main.rs helper2(1);; action ide undo;; sleep 800;; expect-file $L/src/util.rs fn helper(;; expect-file $L/src/main.rs helper(1);; goto $L/src/main.rs 2 4;; sleep 400;; keys al;; wait-completion;; key enter;; wait-editor alpha_item;; action ide actions;; sleep 600;; action ide run-action;; wait-editor // TODO;; shot e2e-lsp;; quit"
# Typing while the server works on a rename: its answer is for the old text, so nothing is
# applied (the file not open in the editor keeps its name on disk too).
S="$WORK/lsp-stale"; mkdir -p "$S/src"
printf 'fn main() {\n    let x = helper(1);\n    println!("{x}");\n}\n' > "$S/src/main.rs"
printf 'pub fn helper(a: i32) -> i32 {\n    a + 1\n}\n' > "$S/src/util.rs"
TC_FAKE_LSP_DELAY_MS=1500 run lsp-stale 60 "sleep 1000;; action add-project $S;; sleep 1200;; open-file $S/src/main.rs;; wait-lsp ready;; goto $S/src/main.rs 1 14;; sleep 400;; action ide rename;; wait-rename;; sleep 300;; keys helper2\\r;; sleep 300;; keys zz;; wait-editor zz;; wait-toast nothing was applied|tidak ada yang diterapkan;; expect-file $S/src/util.rs fn helper(;; expect-not-file $S/src/util.rs helper2;; quit"
# ⌘-click on a symbol goes to its definition; the pointer resting on one shows its hover.
run lsp-click 60 "sleep 1000;; action add-project $L;; sleep 1200;; open-file $L/src/main.rs;; wait-lsp ready;; goto $L/src/main.rs 1 14;; sleep 500;; mouse move editor-caret;; wait-hover;; mouse cmd-move editor-caret;; sleep 200;; mouse cmd-down editor-caret;; mouse cmd-up editor-caret;; wait-active util.rs;; wait-caret 1;; shot e2e-lsp-click;; quit"
# Without the server installed: the status line says so, with how to install it.
LSP=/nonexistent/tc-no-lsp run lsp-missing 40 "sleep 1000;; action add-project $L;; sleep 1200;; open-file $L/src/util.rs;; wait-lsp missing;; expect-rect ide-status;; quit"

# Grid filter (v1.2): "Active" keeps only projects running something or waiting.
run grid-active 50 "sleep 1000;; action add-project $P1;; sleep 900;; action add-project $P2;; sleep 1200;; keys sleep 30\\r;; wait-status running;; sleep 2500;; action grid;; sleep 800;; expect-shown tile:beta;; expect-shown tile:alpha;; click grid:active;; sleep 600;; expect-shown tile:beta;; expect-hidden tile:alpha;; shot e2e-grid-active;; click grid:all;; sleep 600;; expect-shown tile:alpha;; quit"

run grid 60 "sleep 1000;; action add-project $P1;; sleep 900;; action split-right;; sleep 800;; keys echo alpha-left\\r;; action add-project $P2;; sleep 900;; action split-down;; sleep 800;; action add-project $R;; sleep 1200;; action grid;; sleep 1200;; expect-fill 2;; expect-fill 3;; expect-fill 5;; expect-fill 6;; expect-fill 8;; shot e2e-grid;; action focus;; sleep 1200;; expect-fill 8;; action select 0;; sleep 1200;; expect-fill 2;; expect-fill 3;; wait-screen alpha-left;; click view:grid;; sleep 1200;; expect-shown tile:beta;; shot e2e-all-projects;; click view:focus;; sleep 1200;; expect-hidden tile:beta;; click view:grid;; sleep 1200;; expect-shown tile:beta;; click project:beta;; sleep 1200;; expect-project beta;; expect-hidden tile:alpha;; shot e2e-focus-beta;; quit"

run editor 60 "sleep 1000;; action add-project $P1;; sleep 1200;; open-file $P1/notes.txt;; sleep 800;; write-file $P1/notes.txt notes v2 by agent\\n;; wait-editor v2 by agent;; open-file $P1/src/main.rs:2;; sleep 800;; click editor-text;; sleep 300;; key cmd+down;; keys // mine\\r;; wait-editor // mine;; write-file $P1/src/main.rs fn main() { agent(); }\\n;; wait-conflict;; expect-not-file $P1/src/main.rs // mine;; action force-save;; sleep 500;; expect-file $P1/src/main.rs // mine;; quit"

# The sidebar's file tree (v1.1): click a file → the editor shows it; files an agent adds or
# changes appear (and get their git letter) without a refresh; nothing is read per frame.
T="$WORK/tree"; mkdir -p "$T/src"
(cd "$T" && git init -q -b main && printf 'line 1\n' > edit.txt && printf 'fn main() {}\n' > src/main.rs \
    && git add -A && git -c user.name=t -c user.email=t@t commit -qm init)
run explorer 60 "sleep 1000;; action add-project $T;; sleep 1500;; wait-tree edit.txt;; click tree:edit.txt;; wait-open edit.txt;; expect-rect editor-text;; write-file $T/edit.txt line 1 changed by agent\\n;; wait-git edit.txt M;; write-file $T/added.txt new\\n;; wait-tree added.txt;; wait-git added.txt U;; click editor-close;; sleep 400;; click toggle-editor;; wait-open edit.txt;; shot e2e-explorer;; quit"


# Terminal tabs (v1.1): drag a tab along the strip to reorder; drop it on a pane's right half to
# make it a split of the tab in view. Ctrl+D in a shell closes its pane.
run tabs 60 "sleep 1000;; action add-project $P2;; sleep 1200;; action new-tab;; sleep 800;; action new-tab;; sleep 800;; wait-tabs 0:2 1:3 2:4;; mouse down tab:2;; sleep 120;; mouse drag tab:2@0.3,0.5;; sleep 120;; mouse drag tab:1@0.5,0.5;; sleep 120;; mouse drag tab:0@0.2,0.5;; sleep 150;; mouse up tab:0@0.2,0.5;; sleep 400;; wait-tabs 0:4 1:2 2:3;; mouse down tab:1;; sleep 120;; mouse drag tab:1@0.5,0.9;; sleep 120;; mouse drag pane:4@0.7,0.5;; sleep 120;; mouse drag pane:4@0.92,0.5;; sleep 150;; shot e2e-tab-drop;; mouse up pane:4@0.92,0.5;; sleep 500;; wait-tabs 0:4,2 1:3;; quit"

# ⌘F with the keyboard in the editor searches the file (all matches found, Enter steps); `:N`
# goes to line N. ⌘F in a terminal still searches the pane (scenario `find`).
run editor-find 50 "sleep 1000;; action add-project $P1;; sleep 1200;; write-file $P1/find.txt one two one\\nthree\\nfour one\\n;; open-file $P1/find.txt;; sleep 800;; click editor-text;; sleep 300;; key cmd+f;; sleep 300;; keys one;; wait-find 3;; key enter;; sleep 200;; key cmd+f;; sleep 300;; keys :3\\r;; wait-caret 3;; sleep 300;; wait-editor one two one\\nthree\\nfour one\\n;; quit"
# A pane header dragged onto another pane's lower half moves the pane there. A pane made by a
# split starts at its final size: its PTY is never resized afterwards.
run pane-drag 50 "sleep 1000;; action add-project $P2;; sleep 1200;; action split-right;; sleep 1500;; wait-tabs 0:2,3;; expect-resizes 3 0;; mouse down pane-header:2@0.3,0.5;; sleep 120;; mouse drag pane-header:2@0.4,0.6;; sleep 120;; mouse drag pane:3@0.5,0.7;; sleep 120;; mouse drag pane:3@0.5,0.92;; sleep 150;; mouse up pane:3@0.5,0.92;; sleep 500;; wait-tabs 0:3,2;; quit"

# The file tree's own actions: a new file (named in place, then opened), renamed in place (the
# open file follows), and a nested folder.
F="$WORK/files"; mkdir -p "$F"; printf 'x\n' > "$F/a.txt"
run files-actions 50 "sleep 1000;; action add-project $F;; sleep 1500;; wait-tree a.txt;; sleep 300;; click files-button:4;; sleep 400;; keys hello.txt\\r;; wait-open hello.txt;; wait-tree hello.txt;; action rename hello.txt;; sleep 400;; keys renamed\\r;; wait-tree renamed.txt;; wait-open renamed.txt;; click files-button:3;; sleep 400;; keys docs/api\\r;; wait-tree docs;; shot e2e-files;; quit"
run ctrl-d 40 "sleep 1000;; action add-project $P1;; sleep 1200;; action split-right;; sleep 2600;; wait-tabs 0:2,3;; key ctrl+d;; wait-tabs 0:2;; sleep 300;; key ctrl+d;; wait-tabs 0:4;; sleep 600;; key ctrl+d;; wait-tabs 0:5;; quit"
run review 60 "sleep 1000;; action add-project $R;; sleep 1200;; hook 2 claude SessionStart;; sleep 300;; wait-snapshot;; write-file $R/edit.txt line 1\\nline 2 changed\\nline 3\\n;; write-file $R/new.txt brand new\\n;; rm-file $R/gone.txt;; sleep 300;; action review;; sleep 1500;; wait-review Added:new.txt,Deleted:gone.txt,Modified:edit.txt;; quit"

# Shell integration (OSC 133) with the user's own shell configuration: a quick failure stays
# quiet; a long one is reported as failed with its exit code.
run shell-integration 60 "sleep 1000;; action add-project $P1;; sleep 2500;; keys false\\r;; sleep 1000;; wait-status idle;; keys sleep 11; false\\r;; sleep 12500;; wait-status failed;; quit"

run find 40 "sleep 1000;; action add-project $P1;; sleep 1200;; keys seq 1 300\\r;; sleep 800;; action find;; sleep 400;; keys 150;; key enter;; sleep 400;; key esc;; action find-all;; sleep 400;; keys 27;; sleep 800;; key esc;; quit"

echo "---- $PASS passed, $FAIL failed (logs: $WORK)"
[ "$FAIL" -eq 0 ]
