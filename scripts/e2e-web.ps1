# End-to-end checks of the Browser panel on Windows: the scenarios of scripts/e2e-web.sh, with
# WebView2. Run it in the signed-in desktop (not over ssh: a window must show), after
# `cargo build` (target\debug\ternarycode.exe and ternarycode-hook.exe), from the repository:
#
#   powershell -ExecutionPolicy Bypass -File scripts\e2e-web.ps1 [-Only NAME] [-Bin DIR]
param([string]$Only = "", [string]$Bin = "")
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
if (-not $Bin) { $Bin = Join-Path $Root "target\debug" }
$App = Join-Path $Bin "ternarycode.exe"
$Hook = Join-Path $Bin "ternarycode-hook.exe"
$Work = Join-Path $env:TEMP ("tc-e2e-web-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Force $Work | Out-Null
$script:Pass = 0; $script:Fail = 0

function Write-Text($path, $text) { [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding $false)) }

function Run($name, $secs, $steps) {
    if ($Only -and -not $name.Contains($Only)) { return }
    $data = Join-Path $Work "data-$name"
    New-Item -ItemType Directory -Force $data | Out-Null
    $env:TERNARYCODE_DATA_DIR = $data
    $env:TERNARYCODE_SHOT_DIR = $Work
    $env:TERNARYCODE_DEV_SCRIPT = $steps
    $env:TERNARYCODE_DOWNLOAD_DIR = Join-Path $Work "downloads"
    # A virtual machine running the x64 build through emulation is slow: waits are longer.
    $env:TERNARYCODE_DEV_WAIT = "40"
    $log = Join-Path $Work "$name.log"
    $p = Start-Process -FilePath $App -PassThru -RedirectStandardError $log -RedirectStandardOutput "$log.out"
    if (-not $p.WaitForExit($secs * 4000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Start-Sleep 1 }
    Get-Process -Name ternarycode -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    $text = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
    if ($null -eq $text) { $text = "" }
    $p1 = ([regex]::Matches($text, "DEVTEST PASS")).Count
    $f1 = ([regex]::Matches($text, "DEVTEST FAIL")).Count
    $done = ([regex]::Matches($text, "DEVTEST DONE")).Count
    $script:Pass += $p1; $script:Fail += $f1
    if ($f1 -eq 0 -and $done -gt 0) { "ok    $name ($p1 checks)" }
    else {
        "FAIL  $name ($p1 passed, $f1 failed, done=$done)"
        ($text -split "`n") | Where-Object { $_ -match "DEVTEST FAIL" } | ForEach-Object { "        $_" }
        if ($done -eq 0) { $script:Fail += 1 }
    }
}

$P1 = Join-Path $Work "alpha"; New-Item -ItemType Directory -Force $P1 | Out-Null
Write-Text (Join-Path $P1 "upload.txt") "upload me`n"
$Site = Join-Path $Work "site"; New-Item -ItemType Directory -Force $Site | Out-Null
$DL = Join-Path $Work "downloads"; New-Item -ItemType Directory -Force $DL | Out-Null
$Mcp = Join-Path $Work "mcp"; New-Item -ItemType Directory -Force $Mcp | Out-Null
function Call($name, $tool, $argsJson) { Write-Text (Join-Path $Mcp "$name.json") ('{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"' + $tool + '","arguments":' + $argsJson + '}}' + "`n") }
# A preset running an agent's tool calls in its pane (a batch file: `m NAME` pipes NAME.json into
# the MCP server and writes NAME.out, then says ok-NAME).
function Preset($name, $id, [string[]]$calls) {
    $cmd = Join-Path $Work "$id.cmd"
    $lines = @("@echo off") + ($calls | ForEach-Object { "call :m $_" }) + @("goto :eof", ":m", "`"$Hook`" mcp claude < `"$Mcp\%1.json`" > `"$Mcp\%1.out`"", "echo ok-%1", "goto :eof")
    [IO.File]::WriteAllLines($cmd, $lines)
    $d = Join-Path $Work "data-$name"; New-Item -ItemType Directory -Force $d | Out-Null
    $command = ("cmd /c " + $cmd) -replace '\\', '\\'
    Write-Text (Join-Path $d "settings.json") ('{"browser_agent_js":true,"presets":[{"id":"' + $id + '","name":"' + $id + '","command":"' + $command + '"}]}')
}

# A small web server for the pages (no Python on Windows): a process of its own (a job blocked
# waiting for a connection can't be stopped), one request a connection.
$listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
$listener.Start(); $Port = $listener.LocalEndpoint.Port; $listener.Stop()
$serverScript = Join-Path $Work "server.ps1"
Write-Text $serverScript @'
param($port, $site)
$l = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, [int]$port); $l.Start()
while ($true) {
    $c = $l.AcceptTcpClient()
    try {
        $s = $c.GetStream(); $r = New-Object IO.StreamReader($s)
        $line = $r.ReadLine(); while (($h = $r.ReadLine()) -ne "" -and $null -ne $h) {}
        $path = ($line -split " ")[1].Split("?")[0]; if ($path -eq "/") { $path = "/index.html" }
        $file = Join-Path $site ($path.TrimStart("/") -replace "/", "\")
        if (Test-Path $file -PathType Leaf) {
            $body = [IO.File]::ReadAllBytes($file)
            $type = if ($file.EndsWith(".html")) { "text/html; charset=utf-8" } else { "text/plain" }
            $head = "HTTP/1.1 200 OK`r`nContent-Type: $type`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n"
        } else { $body = [Text.Encoding]::UTF8.GetBytes("not found"); $head = "HTTP/1.1 404 Not Found`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n" }
        $hb = [Text.Encoding]::ASCII.GetBytes($head); $s.Write($hb, 0, $hb.Length); $s.Write($body, 0, $body.Length); $s.Flush()
    } catch {} finally { $c.Close() }
}
'@
$server = Start-Process powershell -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $serverScript, $Port, $Site -WindowStyle Hidden -PassThru
Start-Sleep 2
$W = "http://localhost:$Port"

Write-Text "$Site\index.html" '<!doctype html><title>TC Test Page</title><h1>Hello from the dev server</h1><a href="/second.html">Second page</a>'
Write-Text "$Site\second.html" '<!doctype html><title>Second</title><h2>Second page</h2>'
Write-Text "$Site\agent.html" '<!doctype html><title>Agent Page</title><h1>Shop</h1><p id="count">Count: 0</p><script>let n = 0; function add() { n++; document.getElementById("count").textContent = "Count: " + n; console.log("clicked", n); }</script><button onclick="add()">Add one</button><input placeholder="Search"><a href="/second.html">Second page</a> <a href="https://example.com/">Elsewhere</a>'
Write-Text "$Site\file.txt" "hello download`n"
Write-Text "$Site\dialogs.html" '<!doctype html><title>Dialogs</title><button id="rm" onclick="document.title = confirm(&quot;Remove Wool Beanie?&quot;) ? &quot;Removed&quot; : &quot;Kept&quot;">Remove</button><button id="hi" onclick="alert(&quot;Hello there&quot;); document.title = &quot;Alerted&quot;">Hello</button><button id="nm" onclick="document.title = &quot;Name: &quot; + prompt(&quot;Your name?&quot;, &quot;Dina&quot;)">Name</button><a id="dl" href="/file.txt" download>Get the file</a>'
Write-Text "$Site\find.html" '<!doctype html><title>Find Page</title><p>Linen shirt</p><p>Wool beanie</p><p>linen tote and LINEN scarf</p>'
Write-Text "$Site\frame.html" '<!doctype html><title>Pay Frame</title><body style="margin:0"><input id="card" style="position:absolute;left:10px;top:10px;width:200px;height:30px"><button style="position:absolute;left:10px;top:60px;width:100px;height:30px" onclick="parent.postMessage(&quot;paid:&quot; + document.getElementById(&quot;card&quot;).value, &quot;*&quot;)">Pay</button></body>'
Write-Text "$Site\human.html" ('<!doctype html><title>Human Page</title><body style="margin:0;font:14px sans-serif"><form onsubmit="event.preventDefault(); document.title = &quot;Submitted: &quot; + this.q.value"><input name="q" aria-label="Search field" style="position:absolute;left:10px;top:10px;width:150px"><button style="position:absolute;left:170px;top:10px">Search</button></form><div role="button" style="position:absolute;left:10px;top:50px;width:100px;height:30px;background:#ddd" onmouseenter="document.title = &quot;Hovered&quot;">Hover me</div><div id="a" role="button" style="position:absolute;left:10px;top:90px;width:80px;height:30px;background:#cde">Drag A</div><div id="b" role="button" style="position:absolute;left:200px;top:90px;width:80px;height:30px;background:#edc">Target B</div><div role="button" draggable="true" style="position:absolute;left:10px;top:130px;width:80px;height:30px;background:#dec" ondragstart="event.dataTransfer.setData(&quot;text&quot;, &quot;c&quot;)">Card C</div><div role="button" style="position:absolute;left:200px;top:130px;width:80px;height:30px;background:#ced" ondragover="event.preventDefault()" ondrop="document.title = &quot;HTML5 drop &quot; + event.dataTransfer.getData(&quot;text&quot;)">Drop D</div><input type="file" aria-label="Receipt" style="position:absolute;left:10px;top:170px" onchange="document.title = &quot;File: &quot; + this.files[0].name"><iframe title="Pay" src="http://127.0.0.1:' + $Port + '/frame.html" style="position:absolute;left:20px;top:220px;width:320px;height:120px;border:0"></iframe><script>let down = false; document.getElementById("a").addEventListener("pointerdown", () => down = true); document.addEventListener("pointerup", (e) => { const t = document.elementFromPoint(e.clientX, e.clientY); if (down && t && t.id === "b") document.title = "Dropped on B"; down = false; }); addEventListener("message", (e) => { if (String(e.data).startsWith("paid:")) document.title = "Paid: " + String(e.data).slice(5); });</script></body>')
Write-Text "$Site\popup.html" '<!doctype html><title>Popup Opener</title><button id="signin" style="font:30px sans-serif;margin:40px;padding:20px">Sign in</button><script>document.getElementById("signin").onclick = function () { window.open("/signin.html", "signin", "width=480,height=560"); }; window.addEventListener("message", function (e) { if (e.origin === location.origin) document.title = "Signed in: " + e.data; });</script>'
Write-Text "$Site\signin.html" '<!doctype html><title>Sign In Page</title><h1>Sign in</h1><script>setTimeout(function () { window.opener.postMessage("tc-ok", "*"); window.close(); }, 1500);</script>'
Write-Text "$Site\newwin.html" '<!doctype html><title>New Window Page</title><a id="nw" href="/second.html" target="_blank" style="display:block;font:40px sans-serif;padding:30px 20px;height:200px">New window</a>'
Write-Text "$Site\keys.html" '<!doctype html><title>Keys Page</title><input id="k" autofocus style="font:30px sans-serif" oninput="document.title = &quot;Keys: &quot; + this.value">'

Run "web-basic" 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/;; wait-browser TC Test Page;; wait-pages 3*;; sleep 800;; expect-web-placed;; web-shot web-basic-page;; shot web-basic;; action palette;; sleep 600;; expect-web-hidden;; key esc;; sleep 600;; expect-web-placed;; action browser-address;; sleep 300;; keys $W/second.html\r;; wait-browser Second;; action new-browser-tab;; wait-pages 3,4*;; action browser-address;; sleep 300;; keys $W/\r;; wait-browser TC Test Page;; click page-tab:3;; wait-pages 3*,4;; wait-browser Second;; sleep 500;; expect-web-placed;; quit"

Call "nav" "browser_navigate" ('{"url":"localhost:' + $Port + '/agent.html"}')
Call "add" "browser_click" '{"ref":"e1","element":"Add one button"}'
Call "typ" "browser_type" '{"ref":"e2","element":"Search field","text":"linen"}'
Call "con" "browser_console_messages" '{}'
Call "js" "browser_evaluate" '{"function":"() => document.title + \" / \" + n"}'
Call "pic" "browser_take_screenshot" '{}'
Call "sec" "browser_click" '{"ref":"e3","element":"Second page link"}'
Call "bak" "browser_navigate_back" '{}'
Call "scr" "browser_scroll" '{"direction":"down"}'
Call "away" "browser_click" '{"ref":"e4","element":"Elsewhere link"}'
Preset "web-agent" "agent" @("nav", "add", "typ", "con", "js", "pic", "sec", "bak", "scr", "away")
Run "web-agent" 120 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; action run agent;; wait-screen ok-nav;; expect-file $Mcp\nav.out Page: Agent Page;; expect-file $Mcp\nav.out [ref=e1];; wait-screen ok-add;; expect-file $Mcp\add.out Count: 1;; wait-screen ok-typ;; expect-file $Mcp\typ.out linen;; wait-screen ok-con;; expect-file $Mcp\con.out [log] clicked 1;; wait-screen ok-js;; expect-file $Mcp\js.out Agent Page / 1;; wait-screen ok-pic;; expect-file $Mcp\pic.out image/png;; wait-screen ok-sec;; expect-file $Mcp\sec.out Page: Second;; wait-screen ok-bak;; expect-file $Mcp\bak.out Page: Agent Page;; wait-screen ok-scr;; expect-file $Mcp\scr.out Scrolled down.;; wait-shown browser-ask:*;; shot web-agent-ask;; click browser-block:*;; wait-screen ok-away;; expect-file $Mcp\away.out Blocked by the user: example.com;; quit"

Call "hnav" "browser_navigate" ('{"url":"localhost:' + $Port + '/human.html"}')
Call "hq" "browser_click" '{"ref":"e1","element":"Search field"}'
Call "htype" "browser_keyboard_type" '{"text":"hello","submit":true}'
Call "hhover" "browser_hover" '{"ref":"e3","element":"Hover me"}'
Call "hdrag" "browser_drag" '{"startRef":"e4","startElement":"Drag A","endRef":"e5","endElement":"Target B"}'
Call "hdnd" "browser_drag" '{"startRef":"e6","startElement":"Card C","endRef":"e7","endElement":"Drop D"}'
Call "hup" "browser_file_upload" '{"ref":"e8","element":"Receipt","paths":["upload.txt"]}'
Call "hbad" "browser_file_upload" '{"ref":"e8","element":"Receipt","paths":["C:\\Windows\\win.ini"]}'
Call "hshot" "browser_take_screenshot" '{}'
Call "hxy1" "browser_mouse_click_xy" '{"x":130,"y":245,"element":"Card number in the Pay frame"}'
Call "htype2" "browser_keyboard_type" '{"text":"4242"}'
Call "hxy2" "browser_mouse_click_xy" '{"x":80,"y":295,"element":"Pay in the Pay frame"}'
Preset "web-agent-human" "hum" @("hnav", "hq", "htype", "hhover", "hdrag", "hdnd", "hup", "hbad", "hshot", "hxy1", "htype2", "hxy2")
Run "web-agent-human" 120 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; action run hum;; wait-screen ok-htype;; wait-screen ok-hdrag;; wait-screen ok-hbad;; wait-screen ok-hxy1;; wait-screen ok-hxy2;; expect-file $Mcp\hnav.out [ref=e8];; expect-file $Mcp\htype.out Page: Submitted: hello;; expect-file $Mcp\hhover.out Page: Hovered;; expect-file $Mcp\hdrag.out Page: Dropped on B;; expect-file $Mcp\hdnd.out Page: HTML5 drop c;; expect-file $Mcp\hup.out Page: File: upload.txt;; expect-file $Mcp\hbad.out outside the project;; expect-file $Mcp\hshot.out CSS pixels;; expect-file $Mcp\hxy2.out Page: Paid: 4242;; web-shot web-agent-human;; quit"

Run "web-dialogs" 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/dialogs.html;; wait-browser Dialogs;; wait-pages 3*;; sleep 800;; web-click document.getElementById('rm');; wait-dialog Remove Wool Beanie;; expect-shown browser-dialog:3;; shot web-dialog;; click browser-dialog-cancel:3;; wait-browser Kept;; expect-hidden browser-dialog:3;; web-click document.getElementById('rm');; wait-dialog Remove Wool Beanie;; click browser-dialog-ok:3;; wait-browser Removed;; web-click document.getElementById('hi');; wait-dialog Hello there;; click browser-dialog-ok:3;; wait-browser Alerted;; web-click document.getElementById('nm');; wait-dialog Your name;; click browser-dialog-ok:3;; wait-browser Name: Dina;; web-click document.getElementById('dl');; wait-download file.txt;; expect-file $DL\file.txt hello download;; expect-shown browser-downloads:3;; quit"

Call "dnav" "browser_navigate" ('{"url":"localhost:' + $Port + '/dialogs.html"}')
Call "drm" "browser_click" '{"ref":"e1","element":"Remove button"}'
Call "dno" "browser_handle_dialog" '{"accept":false}'
Call "dnm" "browser_click" '{"ref":"e3","element":"Name button"}'
Call "dyes" "browser_handle_dialog" '{"accept":true,"promptText":"Ada"}'
Call "ddl" "browser_click" '{"ref":"e4","element":"Get the file link"}'
Preset "web-agent-dialogs" "dlg" @("dnav", "drm", "dno", "dnm", "dyes", "ddl")
Remove-Item -Force -ErrorAction SilentlyContinue "$DL\file.txt"
Run "web-agent-dialogs" 90 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; action run dlg;; wait-screen ok-ddl;; expect-file $Mcp\drm.out A dialog is open in the page (confirm;; expect-file $Mcp\dno.out Dismissed the dialog;; expect-file $Mcp\dno.out Page: Kept;; expect-file $Mcp\dnm.out A dialog is open in the page (prompt;; expect-file $Mcp\dyes.out Page: Name: Ada;; expect-file $Mcp\ddl.out Downloaded file.txt to;; quit"

Run "web-tools" 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/find.html;; wait-browser Find Page;; wait-pages 3*;; sleep 800;; page-find linen;; wait-page-find 1/3;; expect-shown browser-find:3;; page-find-next;; wait-page-find 2/3;; page-find-next;; wait-page-find 3/3;; page-find-next;; wait-page-find 1/3;; page-find-prev;; wait-page-find 3/3;; shot web-find;; page-find zzz;; wait-page-find 0/0;; page-zoom 1;; sleep 400;; expect-page-zoom 1.1;; expect-shown browser-zoom:3;; click browser-zoom:3;; sleep 400;; expect-page-zoom 1;; page-inspect;; wait-inspector;; quit"

Run "web-popup" 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/popup.html;; wait-browser Popup Opener;; wait-pages 3*;; sleep 1000;; web-click document.getElementById('signin');; wait-browser Signed in: tc-ok;; wait-pages 3*;; action open-web $W/newwin.html;; wait-browser New Window Page;; sleep 800;; web-click document.getElementById('nw');; wait-pages 3,4*;; wait-browser Second;; quit"

# An agent's click that opens a sign-in pop-up: its next steps act in it; its click there signs
# the page in and the pop-up closes; a pop-up to a site the agent may not open is asked first.
Write-Text "$Site\agentpop.html" '<!doctype html><title>Agent Popup Opener</title><button onclick="window.open(&quot;/agentsignin.html&quot;, &quot;s&quot;, &quot;width=480,height=560&quot;)">Sign in</button><button onclick="window.open(&quot;http://tc-popup.invalid/x&quot;, &quot;w&quot;, &quot;width=400,height=400&quot;)">Sign in elsewhere</button><script>window.addEventListener("message", function (e) { if (e.origin === location.origin) document.title = "Signed in: " + e.data; });</script>'
Write-Text "$Site\agentsignin.html" '<!doctype html><title>Agent Sign In</title><button onclick="window.opener.postMessage(&quot;agent-ok&quot;, &quot;*&quot;); window.close();">Continue</button>'
Call "pnav" "browser_navigate" ('{"url":"localhost:' + $Port + '/agentpop.html"}')
Call "pclick" "browser_click" '{"ref":"e1","element":"Sign in button"}'
Call "pcont" "browser_click" '{"ref":"e1","element":"Continue button"}'
Call "pclick2" "browser_click" '{"ref":"e1","element":"Sign in button"}'
Call "pnav2" "browser_navigate" ('{"url":"localhost:' + $Port + '/agentpop.html"}')
Call "pelse" "browser_click" '{"ref":"e2","element":"Sign in elsewhere button"}'
Preset "web-agent-popup" "pop" @("pnav", "pclick", "pcont", "pclick2", "pnav2", "pelse")
Run "web-agent-popup" 90 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; action run pop;; wait-screen ok-pnav;; expect-file $Mcp\pnav.out Page: Agent Popup Opener;; wait-screen ok-pclick;; expect-file $Mcp\pclick.out A pop-up window opened;; expect-file $Mcp\pclick.out Page: Agent Sign In (a pop-up window);; wait-screen ok-pcont;; expect-file $Mcp\pcont.out The pop-up window closed;; expect-file $Mcp\pcont.out Page: Signed in: agent-ok;; wait-screen ok-pclick2;; wait-screen ok-pnav2;; expect-file $Mcp\pnav2.out The pop-up window closed;; expect-file $Mcp\pnav2.out Page: Agent Popup Opener;; wait-shown browser-ask:*;; click browser-block:*;; wait-screen ok-pelse;; expect-file $Mcp\pelse.out Blocked by the user: tc-popup.invalid;; quit"

# The app's keys pressed while a page has the keyboard: typed as a person (SendKeys), Ctrl+F
# opens the page's find bar; other keys stay the page's.
$keys = Join-Path $Work "keys.ps1"
Write-Text $keys @'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -Namespace Tc -Name Keys -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, System.UIntPtr extra);'
Start-Sleep 3
(New-Object -ComObject WScript.Shell).AppActivate('TernaryCode') | Out-Null
Start-Sleep 2
[System.Windows.Forms.SendKeys]::SendWait('abc')
Start-Sleep 1
# Ctrl+F a key at a time, held as a person holds it.
[Tc.Keys]::keybd_event(0x11, 0x1d, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 150
[Tc.Keys]::keybd_event(0x46, 0x21, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100
[Tc.Keys]::keybd_event(0x46, 0x21, 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100
[Tc.Keys]::keybd_event(0x11, 0x1d, 2, [UIntPtr]::Zero)
'@
Run "web-keys" 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/keys.html;; wait-browser Keys Page;; wait-pages 3*;; sleep 800;; web-click document.getElementById('k');; page-focus;; sleep 500;; spawn & '$keys';; wait-browser Keys: abc;; wait-shown browser-find:3;; quit"

# Codex from Run Agent: the settings it gets for the run (hooks, notify, browser tools, guide)
# reach it whole through PowerShell, which in 5.1 drops each `"` inside an argument: Codex lists
# the browser tools' server, and Run Agent starts it. Only where Codex is installed.
if (Get-Command codex -ErrorAction SilentlyContinue) {
    $flags = '-c "$env:TC_CODEX_HOOKS" -c "$env:TC_CODEX_NOTIFY" -c "$env:TC_CODEX_MCP" -c "$env:TC_CODEX_GUIDE"'
    Run "agent-codex" 90 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; keys codex $flags mcp list\r;; wait-screen mcp codex;; expect-screen-count invalid type 0;; keys cls\r;; sleep 1500;; action run-agent codex;; wait-screen OpenAI;; expect-screen-count invalid type 0;; expect-screen-count failed to load 0;; quit"
} else { "skip  agent-codex (Codex is not installed)" }

Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
"== $($script:Pass) checks passed, $($script:Fail) failed (logs in $Work)"
if ($script:Fail -ne 0) { exit 1 }
