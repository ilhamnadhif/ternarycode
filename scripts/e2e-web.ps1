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
# the MCP server and writes NAME.out, then says ok-NAME; "wait N" pauses about N-1 seconds;
# "until NAME" waits for the file NAME in the calls' folder; "read" waits for a line typed in the
# pane). Agents ask before other sites there (Settings) unless -Ask false: the scenarios of the
# question have it.
function Preset($name, $id, [string[]]$calls, [string]$Ask = "true") {
    $cmd = Join-Path $Work "$id.cmd"
    $steps = @(); $n = 0
    foreach ($c in $calls) {
        $n += 1
        if ($c -like "wait *") { $steps += "ping -n " + $c.Split(" ")[1] + " 127.0.0.1 >nul" }
        elseif ($c -like "until *") { $steps += ":u$n"; $steps += "if not exist `"$Mcp\" + $c.Split(" ")[1] + "`" (ping -n 2 127.0.0.1 >nul & goto u$n)" }
        elseif ($c -eq "read") { $steps += "set /p line=" }
        else { $steps += "call :m $c" }
    }
    $lines = @("@echo off") + $steps + @("goto :eof", ":m", "`"$Hook`" mcp claude < `"$Mcp\%1.json`" > `"$Mcp\%1.out`"", "echo ok-%1", "goto :eof")
    [IO.File]::WriteAllLines($cmd, $lines)
    $d = Join-Path $Work "data-$name"; New-Item -ItemType Directory -Force $d | Out-Null
    $command = ("cmd /c " + $cmd) -replace '\\', '\\'
    Write-Text (Join-Path $d "settings.json") ('{"browser_agent_js":true,"browser_agent_ask":' + $Ask + ',"presets":[{"id":"' + $id + '","name":"' + $id + '","command":"' + $command + '"}]}')
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
            $type = if ($file.EndsWith(".html")) { "text/html; charset=utf-8" } elseif ($file.EndsWith(".wav")) { "audio/wav" } else { "text/plain" }
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

# Asked to close the browser, an agent closes its page: the user's page stays, in front (the
# answer names it while the panel shows; on this small screen it floats); with panel the panel
# hides too (the user's page kept); with no page of its own it says so.
Call "cnav" "browser_navigate" ('{"url":"localhost:' + $Port + '/agent.html"}')
Call "ccls" "browser_close" '{}'
Call "cnav2" "browser_navigate" ('{"url":"localhost:' + $Port + '/second.html"}')
Call "cpanel" "browser_close" '{"panel":true}'
Call "cnone" "browser_close" '{}'
Preset "web-agent-close" "cls" @("cnav", "wait 5", "ccls", "wait 5", "cnav2", "wait 5", "cpanel", "wait 5", "cnone") -Ask false
# The user's page is on another site (127.0.0.1, not localhost): the agent opens its own.
Run "web-agent-close" 90 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web http://127.0.0.1:$Port/;; wait-browser TC Test Page;; wait-pages 3*;; wait-screen PS C:;; action run cls;; wait-screen ok-cnav;; expect-file $Mcp\cnav.out Page: Agent Page;; wait-screen ok-ccls;; wait-pages 3*;; expect-file $Mcp\ccls.out Closed your page;; expect-file $Mcp\ccls.out TC Test Page;; wait-screen ok-cnav2;; expect-file $Mcp\cnav2.out Page: Second;; wait-screen ok-cpanel;; wait-panel hidden;; wait-pages 3*;; expect-file $Mcp\cpanel.out The Browser panel is hidden.;; wait-screen ok-cnone;; expect-file $Mcp\cnone.out You had no page open;; quit"

# An agent's first browser call after a prompt shows the Browser panel (it floats on this small
# screen). The user hides it; the agent's next call (no prompt since) leaves it hidden; after the
# user types into the agent's terminal (a prompt), its next call shows it again.
Call "snav" "browser_navigate" ('{"url":"localhost:' + $Port + '/agent.html"}')
Call "ssnap" "browser_snapshot" '{}'
Call "ssnap2" "browser_snapshot" '{}'
Call "ssnapb" "browser_snapshot" '{}'
Preset "web-agent-show" "show" @("snav", "until go", "ssnap", "until go2", "ssnapb", "read", "ssnap2") -Ask false
Run "web-agent-show" 90 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; action run show;; wait-screen ok-snav;; wait-panel float;; action show-browser;; wait-panel hidden;; write-file $Mcp\go go;; wait-screen ok-ssnap;; sleep 1000;; wait-panel hidden;; expect-file $Mcp\ssnap.out Page: Agent Page;; keys typed;; sleep 500;; write-file $Mcp\go2 go;; wait-screen ok-ssnapb;; sleep 1000;; wait-panel hidden;; keys again\r;; wait-screen ok-ssnap2;; wait-panel float;; quit"

# The update dialog (a release from a test feed; Later at launch, then Update in the status bar)
# covers the page: its web view hides under it and comes back after Later.
Write-Text (Join-Path $Work "feed.json") '{"tag_name":"v99.0.0","html_url":"https://example.invalid/releases/v99.0.0","published_at":"2026-11-18T00:00:00Z","body":"Test release","assets":[]}'
$d = Join-Path $Work "data-web-update"; New-Item -ItemType Directory -Force $d | Out-Null
Write-Text (Join-Path $d "settings.json") '{"check_updates":true}'
$env:TERNARYCODE_UPDATE_FEED = "file:///" + ((Join-Path $Work "feed.json") -replace '\\', '/')
Run "web-update" 90 "sleep 1000;; action add-project $P1;; wait-shown update-close;; sleep 800;; key esc;; wait-hidden update-close;; action open-web $W/;; wait-browser TC Test Page;; wait-pages 3*;; sleep 1500;; expect-web-placed;; click update-button;; wait-shown update-close;; sleep 1500;; expect-web-hidden;; shot web-update;; click update-close;; wait-hidden update-close;; sleep 1500;; expect-web-placed;; quit"
Remove-Item Env:TERNARYCODE_UPDATE_FEED -ErrorAction SilentlyContinue

# Pages that sleep out of view and the ones that don't (sound, a form being filled in); the
# tab's speaker mutes a page; Reopen Closed Page. Sleep after 4 s here.
$ms = New-Object IO.MemoryStream; $bw = New-Object IO.BinaryWriter($ms)
$bw.Write([Text.Encoding]::ASCII.GetBytes("RIFF")); $bw.Write([int](36 + 16000)); $bw.Write([Text.Encoding]::ASCII.GetBytes("WAVEfmt ")); $bw.Write([int]16); $bw.Write([int16]1); $bw.Write([int16]1); $bw.Write([int]8000); $bw.Write([int]16000); $bw.Write([int16]2); $bw.Write([int16]16); $bw.Write([Text.Encoding]::ASCII.GetBytes("data")); $bw.Write([int]16000); $bw.Write((New-Object byte[] 16000)); $bw.Flush()
[IO.File]::WriteAllBytes("$Site\tone.wav", $ms.ToArray())
Write-Text "$Site\sound.html" '<!doctype html><title>Sound Page</title><audio id="s" loop src="/tone.wav"></audio><button id="play" onclick="document.getElementById(&quot;s&quot;).play()" style="font:30px sans-serif;margin:40px">Play</button>'
Write-Text "$Site\form.html" '<!doctype html><title>Form Page</title><form><textarea id="msg" name="msg"></textarea></form>'
$env:TERNARYCODE_SLEEP_SECS = "4"
Run "web-page-life" 120 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/sound.html;; wait-browser Sound Page;; wait-pages 3*;; sleep 800;; web-log document.getElementById('s').play().then(() => 'playing');; wait-page 3 audible;; action new-browser-tab;; wait-pages 3,4*;; action browser-address;; sleep 300;; keys $W/form.html\r;; wait-browser Form Page;; sleep 500;; web-log (() => { const m = document.getElementById('msg'); m.value = 'half a message'; m.dispatchEvent(new Event('input', { bubbles: true })); return 'typed'; })();; wait-page 4 editing;; action new-browser-tab;; wait-pages 3,4,5*;; action browser-address;; sleep 300;; keys $W/second.html\r;; wait-browser Second;; click panel-hide;; wait-panel hidden;; wait-page 5 asleep;; sleep 2000;; wait-page 3 awake;; wait-page 4 awake;; action show-browser;; click page-tab:3;; wait-pages 3*,4,5;; wait-shown page-mute:3;; click page-mute:3;; wait-page 3 muted;; click page-mute:3;; wait-page 3 unmuted;; wait-page 3 audible;; click page-tab:5;; wait-browser Second;; click page-close:5;; wait-page 5 gone;; action reopen-page;; wait-browser Second;; wait-page front user;; quit"
Remove-Item Env:TERNARYCODE_SLEEP_SECS

# An agent works in a page of the user's already on the site it opens (idle); browser_tabs;
# another page playing sound; its terminal closed.
Call "bnav" "browser_navigate" ('{"url":"localhost:' + $Port + '/second.html"}')
Call "blist" "browser_tabs" '{"action":"list"}'
Call "bnew" "browser_tabs" ('{"action":"new","url":"localhost:' + $Port + '/sound.html"}')
Call "bplay" "browser_click" '{"ref":"e1","element":"Play button"}'
Call "bsel" "browser_tabs" '{"action":"select","index":1}'
Call "bnav2" "browser_navigate" ('{"url":"localhost:' + $Port + '/sound.html"}')
Call "bplay2" "browser_click" '{"ref":"e1","element":"Play button"}'
Call "bcls" "browser_tabs" '{"action":"close","index":2}'
Call "bnew2" "browser_tabs" ('{"action":"new","url":"localhost:' + $Port + '/agent.html"}')
Call "bnew3" "browser_tabs" ('{"action":"new","url":"localhost:' + $Port + '/form.html"}')
Preset "web-agent-tabs" "tabs" @("bnav", "blist", "bnew", "bplay", "bsel", "bnav2", "bplay2", "bcls", "bnew2", "bnew3") -Ask false
Run "web-agent-tabs" 150 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/;; wait-browser TC Test Page;; wait-pages 3*;; wait-screen PS C:;; action run tabs;; wait-screen ok-bnav;; expect-file $Mcp\bnav.out open already (tab 1;; expect-file $Mcp\bnav.out Page: Second;; wait-pages 3*;; wait-page 3 user;; wait-screen ok-blist;; expect-file $Mcp\blist.out your tools act here;; wait-screen ok-bnew;; expect-file $Mcp\bnew.out Page: Sound Page;; wait-screen ok-bsel;; expect-file $Mcp\bsel.out Page: Second;; wait-screen ok-bplay2;; expect-file $Mcp\bplay2.out Also playing sound: tab 2;; wait-screen ok-bcls;; expect-file $Mcp\bcls.out Closed your page;; wait-screen ok-bnew3;; expect-file $Mcp\bnew3.out Page: Form Page;; wait-page-count 3;; action close-pane;; wait-page-count 2;; wait-page front user;; wait-browser Form Page;; quit"

# An agent opens another site at once (no question); its page asleep a while after its agent
# ended closes (3 s here); its page in view stays.
Call "enav" "browser_navigate" ('{"url":"localhost:' + $Port + '/second.html"}')
Call "eext" "browser_navigate" '{"url":"http://tc-popup.invalid/x"}'
Call "enew" "browser_tabs" ('{"action":"new","url":"localhost:' + $Port + '/agent.html"}')
Preset "web-agent-ended" "ended" @("enav", "eext", "enew") -Ask false
$env:TERNARYCODE_SLEEP_SECS = "4"; $env:TERNARYCODE_AGENT_PAGE_SECS = "3"
Run "web-agent-ended" 90 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; action run ended;; wait-screen ok-eext;; expect-file $Mcp\eext.out tc-popup.invalid;; expect-hidden browser-ask:*;; wait-screen ok-enew;; wait-browser Agent Page;; wait-page-count 2;; sleep 5000;; wait-page-count 1;; wait-page front agent;; wait-page front awake;; quit"
Remove-Item Env:TERNARYCODE_SLEEP_SECS, Env:TERNARYCODE_AGENT_PAGE_SECS

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

# The user's keyboard moved to a terminal by a key (not a click): the page they typed in lets go
# of it, and the user's real keys (SendKeys) go to the terminal, not into the page (it once kept
# the keyboard, and the window coming to the front gave it back to the page).
$type = Join-Path $Work "type.ps1"
Write-Text $type @'
Add-Type -AssemblyName System.Windows.Forms
Start-Sleep 2
(New-Object -ComObject WScript.Shell).AppActivate('TernaryCode') | Out-Null
Start-Sleep 2
[System.Windows.Forms.SendKeys]::SendWait('echo kept-${(}40{+}2{)}{ENTER}')
'@
Run "web-focus" 60 "sleep 1000;; action add-project $P1;; sleep 1500;; action open-web $W/keys.html;; wait-browser Keys Page;; wait-pages 3*;; sleep 800;; web-click document.getElementById('k');; page-focus;; wait-page-focus;; sleep 600;; expect-page-keyboard yes;; focus-pane 0;; sleep 800;; expect-page-focus no;; expect-page-keyboard no;; spawn & '$type';; wait-screen kept-42;; wait-web document.getElementById('k').value === '' => true;; quit"

# Codex from Run Agent: the settings it gets for the run (hooks, notify, browser tools, guide)
# reach it whole through PowerShell, which in 5.1 drops each `"` inside an argument: Codex lists
# the browser tools' server, and Run Agent starts it. Only where Codex is installed.
if (Get-Command codex -ErrorAction SilentlyContinue) {
    $flags = '-c "$env:TC_CODEX_HOOKS" -c "$env:TC_CODEX_NOTIFY" -c "$env:TC_CODEX_MCP" -c "$env:TC_CODEX_GUIDE"'
    Run "codex-flags" 90 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; keys codex $flags mcp list\r;; wait-screen mcp codex;; expect-screen-count invalid type 0;; keys cls\r;; sleep 1500;; action run-agent codex;; wait-screen OpenAI;; expect-screen-count invalid type 0;; expect-screen-count failed to load 0;; quit"
} else { "skip  codex-flags (Codex is not installed)" }

# Run Agent starts each agent and its own screen opens: the pane's program is that agent, still
# running seconds later (it did not stop at once on an error), and nothing "not recognized". The
# agents run as installed here, with your sign-ins; nothing is typed into them, but a first run
# makes an agent's usual files. Only the agents installed here.
foreach ($pair in @("claude:claude", "codex:codex", "opencode:opencode", "antigravity:agy")) {
    $id, $program = $pair -split ":"
    if (Get-Command $program -ErrorAction SilentlyContinue) {
        Run "run-agent-$id" 60 "sleep 1000;; action add-project $P1;; sleep 1500;; wait-screen PS C:;; action run-agent $id;; wait-agent $id;; sleep 6000;; wait-agent $id;; expect-screen-count not recognized 0;; shot agent-$id;; quit"
    } else { "skip  run-agent-$id ($program is not installed)" }
}

Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
"== $($script:Pass) checks passed, $($script:Fail) failed (logs in $Work)"
if ($script:Fail -ne 0) { exit 1 }
