<p align="center">
  <img src="assets/icon_256.png" width="112" alt="Ikon TernaryCode">
</p>

<h1 align="center">TernaryCode</h1>

<p align="center">
  <b>Satu jendela untuk semua project Anda dan AI coding agent yang bekerja di dalamnya.</b><br>
  Workspace native berbasis terminal untuk macOS, Windows, dan Linux, seluruhnya ditulis dengan Rust.
</p>

<p align="center">
  <a href="https://github.com/ilhamnadhif/ternarycode/releases/latest"><img alt="Rilis terbaru" src="https://img.shields.io/github/v/release/ilhamnadhif/ternarycode?label=rilis&color=1f5fbf"></a>
  <a href="https://github.com/ilhamnadhif/ternarycode/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ilhamnadhif/ternarycode/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="Platform" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux-native-555">
  <a href="LICENSE"><img alt="Lisensi MIT" src="https://img.shields.io/badge/lisensi-MIT-555"></a>
</p>

<p align="center">
  <a href="#mulai-cepat">Mulai cepat</a> ·
  <a href="#instalasi">Instalasi</a> ·
  <a href="#fitur">Fitur</a> ·
  <a href="#shortcut-keyboard">Shortcut</a> ·
  <a href="#kalau-ada-masalah">Kalau ada masalah</a> ·
  <a href="#build-dari-source">Build</a> ·
  <a href="README.md">English</a>
</p>

![TernaryCode: agent memeriksa perubahannya di panel Browser di samping terminalnya, sementara sidebar menampilkan agent yang menunggu di project lain](docs/screenshots/focus.png)

Menjalankan Claude Code, Codex, Antigravity CLI, atau OpenCode di lima project sekaligus biasanya
berarti lima jendela editor, masing-masing dengan agent tersembunyi di panel terminal, tanpa tahu
mana yang sedang menunggu Anda. TernaryCode menaruh semua project di **satu jendela**: masing-masing
dengan terminal, tab, dan split pane sendiri, **Grid** yang menampilkan semuanya sekaligus, dan
satu pertanyaan yang dijawab cepat dan jujur: **agent mana yang butuh saya sekarang?**

Aplikasinya satu binary native kecil (tanpa Electron atau Node.js; browser-nya memakai WebKit
bawaan macOS): jendela muncul sekitar 0,2 detik, CPU 0 % saat diam, dan sepuluh pane terbuka
memakai sekitar 70 MB.

> [!NOTE]
> **TernaryCode masih muda (0.x) dan berkembang cepat.** macOS adalah platform yang paling banyak
> diuji: semua tes end-to-end berjalan di sana. Build Windows dan Linux lebih baru; panel browser-nya
> menjalankan tes browser yang sama (`scripts/e2e-web.sh`, `scripts/e2e-web.ps1`). Laporan bug dan ide sangat diterima di
> [Issues](https://github.com/ilhamnadhif/ternarycode/issues).

## Sorotan

- **Tahu agent mana yang butuh Anda.** Setiap pane menampilkan *Butuh input*, *Berjalan*,
  *Selesai*, atau *Gagal*, dari hook agent itu sendiri, bukan tebakan. `⌘J` membuka yang paling
  mendesak.
- **Semua project di satu jendela**, masing-masing dengan terminal, tab, dan split sendiri.
  **Grid** menampilkan semuanya secara live, berdampingan.
- **Browser yang bisa dipakai agent** (macOS). Agent membuka dev server Anda di samping
  terminalnya, mengklik halamannya, dan membaca console-nya. Anda melihat setiap langkahnya dan bisa
  mengambil alih kapan saja.
- **Editor dengan language server Anda**, sekali klik saat Anda ingin membaca kodenya.
- **Dokumen langsung terbuka**: PDF, Word, PowerPoint, Excel, CSV, dan gambar.
- **Review apa yang diubah agent**, dan beri setiap agent **git worktree sendiri** saat beberapa
  agent bekerja di satu repository.
- **Bekerja dengan** Claude Code, Codex, Antigravity CLI, dan OpenCode, atau perintah apa pun yang
  Anda jalankan di terminal.

## Mulai cepat

1. [Pasang TernaryCode](#instalasi) lalu buka.
2. **Tambahkan project**: klik **+** di samping *Projects*, drop folder ke sidebar, atau jalankan
   `ternarycode .` di terminal.
3. Klik **Run Agent** (`⇧⌘A`) lalu pilih Claude Code, Codex, Antigravity CLI, atau OpenCode. Agent
   berjalan di terminal tempat Anda berada (atau di tab baru kalau terminal itu sedang dipakai) dan
   melaporkan statusnya ke TernaryCode untuk sesi itu.
4. Lanjutkan pekerjaan lain. Saat agent menunggu Anda, project-nya menampilkan **Butuh input** dan
   status bar menyebut namanya: `⌘J` membawa Anda ke sana, `⇧⌘J` kembali.
5. Opsional: **Settings → Integrations** memasang hook sekali, supaya agent yang Anda jalankan
   sendiri juga melaporkan statusnya.

Agent-nya harus dipasang sendiri (`claude`, `codex`, `agy`, atau `opencode`); TernaryCode
menjalankannya, tidak membawanya. Kalau belum terpasang, Run Agent memberi tahu dan menampilkan
perintah pemasangannya, dengan tombol **Install in Terminal**.

## Instalasi

Unduh dari [rilis terbaru](https://github.com/ilhamnadhif/ternarycode/releases/latest):

| Sistem | Unduhan |
| --- | --- |
| macOS 12 Monterey atau lebih baru (Apple Silicon dan Intel) | installer satu baris di bawah, atau `TernaryCode-<versi>.dmg` |
| Windows 10 (1809) atau lebih baru, x64 | `TernaryCode-<versi>-windows-x64-setup.exe`, atau `-windows-x64.zip` portabel |
| Debian, Ubuntu, Mint, Pop!_OS | `TernaryCode-<versi>-linux-amd64.deb` (`-arm64.deb` untuk ARM) |
| Fedora, RHEL, Rocky, AlmaLinux, openSUSE | `TernaryCode-<versi>-linux-x86_64.rpm` (`-aarch64.rpm`) |
| Linux lainnya (Arch, Manjaro…) | `TernaryCode-<versi>-linux-x86_64.AppImage` atau `.tar.gz` (`-aarch64` untuk ARM) |

### macOS

```sh
curl -fsSL https://raw.githubusercontent.com/ilhamnadhif/ternarycode/main/scripts/install.sh | bash
```

Skrip ini mengunduh rilis terbaru, memeriksa SHA-256 dan tanda tangan kodenya, memasang
`TernaryCode.app` ke `/Applications`, lalu membukanya. Jalankan lagi untuk memperbarui, atau beri
versi tertentu: `… | bash -s -- v0.1.0`.

Bisa juga unduh `.dmg`, buka, lalu seret **TernaryCode** ke **Applications**.

> [!IMPORTANT]
> **“TernaryCode” Not Opened / “Apple could not verify…”?** Rilis belum dinotarisasi Apple, jadi
> macOS memblokir app yang diunduh lewat browser. Pakai installer satu baris di atas (unduhan lewat
> `curl` tidak diblokir), atau klik **Done**, buka **System Settings → Privacy & Security**, lalu
> klik **Open Anyway** di samping “TernaryCode was blocked…”, atau jalankan
> `xattr -dr com.apple.quarantine /Applications/TernaryCode.app`.

### Windows

Jalankan **`TernaryCode-<versi>-windows-x64-setup.exe`**. Setup memasang untuk user Anda tanpa hak
administrator (pilih **Install for all users** di jendela pertamanya untuk memasang ke Program
Files), menambahkan TernaryCode ke Start menu, dan bisa menaruh `ternarycode` di PATH sehingga
`ternarycode .` membuka folder saat ini sebagai project (nyala secara default). Atau ekstrak **`.zip`**
portabel di mana saja.

Pane menjalankan PowerShell 7 (`pwsh`) kalau terpasang, kalau tidak Windows PowerShell, dengan
shell integration yang dimuat tanpa mengubah profil atau execution policy Anda. Agent berjalan
seperti di terminal lain (Claude Code, Codex, Antigravity CLI, dan OpenCode berjalan native di Windows),
dan hook-nya sampai ke app lewat named pipe.

Jendelanya punya title bar TernaryCode sendiri, seperti app Windows pada umumnya: nama app dan
menunya (File, Edit, View, Agent, Window, Help, sama dengan menu bar di Mac, lengkap dengan
shortcut-nya) di kiri, tombol minimize, maximize, dan close di kanan. Seret untuk memindahkan
jendela (dan menempelkannya ke sisi layar), klik dua kali untuk maximize, klik kanan untuk menu
jendela. Setiap pane baru membaca `PATH` Windows yang terbaru, jadi program yang dipasang selama
TernaryCode berjalan langsung ditemukan tanpa restart, seperti di Windows Terminal. Font
terminalnya Cascadia Mono kalau terpasang (Windows 11), kalau tidak Consolas.

> [!IMPORTANT]
> **“Windows protected your PC”?** Build belum ditandatangani kode, jadi SmartScreen memperingatkan
> unduhan baru. Klik **More info**, lalu **Run anyway**.

### Linux

TernaryCode berjalan di X11 dan Wayland dengan OpenGL (renderer software juga bisa) dan butuh glibc
2.28 atau lebih baru (RHEL 8, Debian 10, Ubuntu 18.10, dan yang lebih baru).

```sh
sudo apt install ./TernaryCode-<versi>-linux-amd64.deb                 # Debian, Ubuntu, Mint, Pop!_OS
sudo dnf install ./TernaryCode-<versi>-linux-x86_64.rpm                # Fedora, RHEL, Rocky, AlmaLinux
chmod +x TernaryCode-<versi>-linux-x86_64.AppImage && ./TernaryCode-<versi>-linux-x86_64.AppImage
tar -xzf TernaryCode-<versi>-linux-x86_64.tar.gz -C ~/.local --strip-components=1   # lalu: ternarycode
```

### Membuka project dari terminal

```sh
ternarycode .              # folder ini, sebagai project
ternarycode ~/code/toko    # folder mana pun
ternarycode toko           # project bernama "toko" (atau folder "toko" di samping project Anda)
```

Project terbuka di jendela yang sedang berjalan (TernaryCode dijalankan kalau belum, terpisah dari
terminal, jadi menutup terminal tidak ikut menutup app). Perintah ini berfungsi di setiap pane
TernaryCode; untuk terminal lain, TernaryCode menaruhnya di `PATH` Anda saat dibuka di macOS
(`bin` Homebrew, `/usr/local/bin`, atau `~/.local/bin`, mana yang bisa Anda tulis; kalau tidak,
**Settings → General → Command line**), paket Linux memasangnya, dan setup Windows menambahkannya ke
`PATH`.

### Pembaruan

TernaryCode memeriksa rilis baru di GitHub saat dibuka dan setiap beberapa jam selama berjalan.
Kalau ada, jendela pembaruan menampilkan yang baru; **Install Update** mengunduhnya, memverifikasi
ukuran dan SHA-256 yang dipublikasikan GitHub (dan di macOS juga bundle id, versi, dan tanda
tangan kode), lalu memulai ulang ke versi baru dengan project dan tab Anda dipulihkan (di macOS dan
dengan `.tar.gz` Linux, program di pane Anda, termasuk agent, tetap berjalan selama itu).
Pemeriksaan berikutnya tidak mengganggu: muncul item **Update** di status bar.

App macOS, setup Windows, AppImage, dan `.tar.gz` memperbarui dirinya sendiri; `.deb`, `.rpm`, dan
`.zip` portabel diperbarui dengan memasang paket baru. Matikan pemeriksaan otomatis di
**Settings → General**, atau pakai **Check for Updates…** kapan saja.

### Uninstall

1. Kalau Anda memasang hook agent, hapus dulu di **Settings → Integrations** (**Uninstall** di
   setiap agent mengembalikan file config-nya). Di macOS, **Settings → Browser → Logins &
   cookies → Clear…** menghapus login browser.
2. Hapus app-nya:
   - **macOS**: pindahkan `/Applications/TernaryCode.app` ke Trash, lalu hapus link `ternarycode`
     yang dibuatnya saat dibuka (di `bin` Homebrew, `/usr/local/bin`, `~/.local/bin`, atau
     `~/bin`).
   - **Windows**: **Settings → Apps → Installed apps → TernaryCode → Uninstall** (atau hapus
     folder portabelnya).
   - **Linux**: `sudo apt remove ternarycode` atau `sudo dnf remove ternarycode`; untuk `.tar.gz`,
     hapus `~/.local/bin/ternarycode`, `~/.local/bin/ternarycode-hook`, dan
     `~/.local/share/applications/ternarycode.desktop`; untuk AppImage, hapus file-nya dan link
     `~/.local/bin/ternarycode`.
3. Hapus pengaturan dan workspace kalau tidak ingin dipakai lagi:
   `~/Library/Application Support/ternarycode` (macOS), `%APPDATA%\ternarycode` (Windows), atau
   `~/.local/share/ternarycode` (Linux).

## Fitur

### Project, terminal, dan agent

- **Project adalah folder.** Terminalnya mulai di sana, file dan perubahan git-nya tampil di
  sampingnya, dan sidebar menunjukkan jenisnya (Rust, Go, TypeScript, React, Flutter, Python…),
  branch, jumlah perubahan, dan apa yang sedang dikerjakan agent-nya. Project yang sedang
  menjalankan sesuatu atau menunggu dikelompokkan di bawah **Active** di atas sidebar (bisa
  dimatikan di Settings → General); sisanya diurutkan menurut nama.
- **Tab dan split pane per project**, bisa diseret untuk diatur ulang, dan dipulihkan saat app
  dibuka lagi. Shell yang dipulihkan baru berjalan saat Anda melihatnya, jadi workspace berisi dua
  puluh project terbuka secepat satu project.
- **Jalankan agent** dari preset (Claude Code, Codex, Antigravity CLI, OpenCode, atau perintah Anda
  sendiri): di terminal tempat Anda berada kalau ia sedang menunggu di prompt kosong, selain itu di
  tab baru atau split. Pane agent yang bisa melanjutkan percakapan menawarkan **Resume** setelah
  restart.
- **Satu worktree per agent**, saat beberapa agent bekerja di satu repository: **Run Agent →
  Worktree baru** membuat git worktree dengan branch sendiri (`tc/‹nama›`, di samping repository)
  dan menjalankan agent di sana, jadi agent tidak saling menimpa file. Worktree tampil di bawah
  project-nya, di-review sejak bercabang, di-merge lewat perintah yang diketik di terminal project,
  dan dihapus beserta foldernya (dan branch-nya bila sudah di-merge).
- **Terminal sungguhan**: warna 24-bit, mouse reporting, synchronized output, IME, box drawing yang
  presisi, dan pencarian scrollback di satu pane (`⌘F`) atau semua pane (`⇧⌘F`).
- **Link dibuka dengan `⌘`-klik atau `Ctrl`-klik** (Ctrl-klik di Windows dan Linux), dari tile
  Grid juga. Alamat web terbuka di panel Browser project (dengan `⌥`, atau di Windows dan Linux,
  di browser default Anda), termasuk yang terpotong ke beberapa baris, yang ada dalam tanda kurung,
  dan yang ada di program yang memakai mouse. Hyperlink OSC 8 juga bisa dibuka. `path:baris`
  terbuka di editor. Klik kanan link untuk membuka atau menyalinnya.
- **Drop file ke terminal** dan path-nya langsung terketik di sana, jadi agent bisa membaca
  screenshot. Folder yang di-drop ke sidebar menjadi project. **`⌘V` dengan gambar** di
  clipboard diteruskan ke Claude Code, Codex, Antigravity CLI, atau OpenCode.
- **Server lokal langsung kelihatan.** Kalau pane mencetak `http://localhost:5173` (Vite,
  Next.js, Rails, Django…), alamatnya muncul di header project dan tile Grid selama servernya
  menjawab. Klik untuk membuka halamannya di panel Browser.
- **Shell integration** (zsh, bash, fish, dan PowerShell) menandai prompt dan perintah, jadi
  perintah yang gagal atau selesai setelah lama tetap ketahuan, dan resize tidak pernah
  meninggalkan potongan prompt.

### Tahu agent mana yang butuh Anda

![Grid: enam project berdampingan, dua agent menunggu izin dan satu selesai](docs/screenshots/grid.png)

- Status tiap pane berasal dari, urut dari yang paling dipercaya: **hook** agent (dipasang per
  agent di Settings → Integrations, atau ditambahkan per peluncuran tanpa mengubah config Anda),
  notifikasi terminal (OSC 9, 99, dan 777), shell integration (OSC 133), dan proses foreground.
  Setiap status menyebut sumbernya; tanpa sinyal yang bisa dipercaya, ia hanya menyebut agent-nya
  *Aktif*, bukan menebak.
- **Butuh input**, **Berjalan**, **Selesai**, **Gagal**, dan **Diam** tampil di sidebar, tab, tile
  Grid, dan status bar. **Menunggu kamu**, di atas sidebar (`⌥⌘J`), menampilkan agent yang
  menunggu Anda atau sudah selesai.
- `⌘J` membuka agent paling mendesak yang menunggu, `⇧⌘J` kembali. Fokus keyboard tidak pernah
  pindah sendiri.
- Notifikasi desktop (macOS, Linux) dan badge Dock (macOS) hanya muncul kalau Anda tidak sedang
  melihatnya.
- **Grid** (`⌘0`, atau **Focus | Grid** di atas sidebar) menampilkan semua project secara live,
  dengan terminal yang memenuhi tile-nya. **Active** dan **Waiting** di header-nya hanya
  menampilkan project yang sedang menjalankan sesuatu atau menunggu. Project dengan beberapa tab
  menampilkan tab yang butuh Anda (atau tempat agent bekerja), tanpa pernah berpindah dari tab
  yang sedang Anda ketik; tab-tabnya ada di header tile, untuk dipilih sendiri.

| Agent | Per peluncuran (tanpa mengubah file) | Dipasang sekali |
| --- | --- | --- |
| Claude Code | `claude --mcp-config "$TC_CLAUDE_MCP" --settings "$TC_CLAUDE_SETTINGS"` menambah hook dan tool browser untuk sesi itu | hook di `~/.claude/settings.json` |
| Codex | `codex -c "$TC_CODEX_HOOKS" -c "$TC_CODEX_NOTIFY" -c "$TC_CODEX_MCP" -c "$TC_CODEX_GUIDE"` | blok terkelola di `~/.codex/config.toml`, termasuk tool browser |
| Antigravity CLI | — | belum ada: statusnya “Aktif” (Antigravity CLI menggantikan Gemini CLI; hook Gemini di `~/.gemini/settings.json` tetap bisa dipasang selama preset `gemini` disimpan) |
| OpenCode | tool browser, lewat `$OPENCODE_CONFIG_CONTENT` di setiap pane | plugin `~/.config/opencode/plugins/ternarycode.js` |

Pemasangan menampilkan perubahan persisnya lebih dulu dan menyimpan backup; **Uninstall**
mengembalikan file byte demi byte kalau Anda belum mengubahnya sejak itu. Tool browser ada di semua
sistem (`--mcp-config`, `$TC_CODEX_MCP`, dan `$OPENCODE_CONFIG_CONTENT`), disertai
pesan untuk Codex dan OpenCode, yang tidak membaca instruksi dari server tool, bahwa browser
TernaryCode adalah browser Anda (setelah instruksi Anda sendiri). Settings → Browser menunjukkan
agent mana yang sudah memilikinya.

### Browser untuk Anda dan agent Anda

Setiap project punya **panel Browser** di kanan terminalnya, di tempat laci Code (keduanya
bergantian di sana). Halamannya memakai web view bawaan sistem: WebKit di macOS, WebView2 (mesin
Edge) di Windows, WebKitGTK di Linux. Di Windows dan Linux, tombol-tombol di bawah memakai
`Ctrl+Shift` di tempat `⌘` macOS, dan di dalam halaman `Ctrl+F`, `Ctrl+L`, `Ctrl+R`, `Ctrl+=`, dan
`Ctrl+−` bekerja seperti di browser.

- **Halaman terbuka di sana**: globe di top bar atau `⇧⌘B` menampilkan dan menyembunyikan panel,
  `⌘L` mengetik alamat, `⌥⇧⌘B` membuka halaman baru, `⌘R` memuat ulang, dan `⌘[` `⌘]` mundur dan
  maju. Chip server, link yang di-`⌘`-klik, **Open in Browser** pada file di Files,
  `ternarycode browse <url>`, dan program yang membuka browser dari pane (`$BROWSER`,
  `open https://…`, `vite --open`) semuanya membuka halamannya di sana. Tahan `⌥` untuk browser
  default Anda.
- **Agent memeriksa hasil kerjanya sendiri di sana.** Agent yang dijalankan dari TernaryCode
  mendapat tool browser-nya (membuka halaman, klik, klik ganda, mengetik, memilih opsi, menekan
  tombol, hover, drag, scroll, menunggu, membaca halaman, screenshot, membaca console, menjawab
  dialog, upload file). Klik dan ketikannya adalah event mouse dan keyboard sungguhan, sama seperti
  milik Anda: sampai ke dalam frame situs lain (form kartu, captcha; agent mengkliknya di titik
  halaman), membuka pemilih file dan popup, dan bisa mengetik di editor apa pun. Minta agent memeriksa perubahannya di
  browser dan halamannya terbuka di sampingnya, berbingkai biru, dengan bar yang menyebut apa yang
  sedang ia lakukan (“Clicking the Add to cart button”). Klik atau ketik di halaman untuk
  **Take Over**; **Hand Back** membiarkan agent melanjutkan.
- **Bekerja seperti browser biasa.** Popup login (“Continue with Google”, “Sign in with Apple”)
  terbuka di jendela kecil di atas app, tetap terhubung ke halamannya, dan menutup sendiri setelah
  selesai; title bar-nya menyebut situsnya. `alert()`, `confirm()`, dan `prompt()` tampil sebagai bar
  di atas halamannya. Kamera dan mikrofon bertanya dulu untuk tiap situs. Download masuk folder
  Downloads dan tercantum di toolbar halaman. `⌘F` mencari di halaman, `⌘=` dan `⌘−` memperbesar dan
  memperkecil, dan menu `⋯` halaman punya **Inspect Element** (inspector bawaan web view) dan **Print…**.
  Agent diberi tahu dialog atau popup yang dibuka kliknya dan bekerja di dalamnya (tidak pernah
  mengetik password Anda: ia meminta Anda), mendapat lokasi file yang diunduh, dan hanya meng-upload
  file dari folder project atau Downloads.
- **Satu halaman per agent.** Dua agent di satu tab masing-masing mendapat halaman sendiri,
  bertanda logo agent-nya di tab-nya. Halaman yang di depan mengikuti terminal yang Anda klik; agent yang
  bekerja di halaman di belakangnya memberi titik biru di sana. Link yang Anda buka tidak pernah
  mengambil halaman agent. Minta agent menutup browser atau tab-nya, dan ia menutup halamannya
  (serta menyembunyikan panel kalau Anda minta).
- **Terminal tidak pernah diperas.** Terminal berdampingan yang jadi terlalu sempit di samping panel
  ditampilkan bertumpuk selama panel terbuka (**Keep Side by Side** kalau Anda lebih suka tetap
  berdampingan); kalau tetap tidak muat, panel mengambang di atasnya sampai Anda mengklik terminal.
  `⤢` membuat halaman memenuhi jendela.
- **Agent tetap di jalur aman.** Agent membuka alamat mesin ini (localhost dan sejenisnya) dan situs
  yang Anda izinkan, untuk semua project atau satu project; situs lain menunggu **Allow Once**,
  **Always**, atau **Block**, ditanyakan di TernaryCode bersama pertanyaan lain yang menunggu Anda.
  JavaScript milik agent sendiri (`browser_evaluate`) mati sampai Anda mengizinkannya di
  Settings → Browser. Halaman hanya mendapat keyboard saat Anda mengkliknya.
- **Di Linux** halaman tampil lewat program pembantu, `ternarycode-web`, yang butuh WebKitGTK 4.1
  (`libwebkit2gtk-4.1-0`; paket `.deb` dan `.rpm` menyarankannya). Ia menaruh halaman di jendela
  X11: di Wayland, jendela app berjalan lewat XWayland supaya halaman bisa tampil
  (`TERNARYCODE_WAYLAND=1` mempertahankan jendela Wayland, dan halaman lalu terbuka di browser
  default Anda, seperti tanpa WebKitGTK). `xdg-open https://…` yang diketik di terminal juga membuka
  halamannya di sini.
- **Di Grid**, tile project menampilkan panelnya di samping terminal, halamannya diperkecil supaya
  tata letaknya sama dengan di Focus, dan tetap hidup; tile kecil menampilkan gambar halaman yang
  sedang dipakai agent.
- Satu profil login untuk semua project, tetap tersimpan melewati restart dan pembaruan. Warning dan
  error di console halaman tampil di toolbar-nya. Halaman yang tidak terlihat dan tidak dipakai
  tidur setelah 10 menit (tidak pernah selama agent memakainya), dan status bar menampilkan memori
  yang dipakai halaman.

### Editor dan fitur bahasa

![Laci Code: file Rust terbuka di samping agent yang sedang mengubahnya](docs/screenshots/editor.png)

- Tab, nomor baris, warna sintaks, pencarian, dan penyimpanan aman: tidak pernah menimpa file yang
  diubah agent saat Anda punya perubahan yang belum disimpan (Anda yang memilih).
- Dengan language server terpasang: go to definition (`F12` atau `⌘`-klik), type definition,
  implementation, referensi (`⇧F12`), rename lintas file (`F2`, bisa di-undo sekali), code action
  dan quick fix (`⌘.`), autocomplete, hover, signature help, simbol dokumen dan workspace, problem
  (`⇧⌘M`), dan format saat menyimpan. Editor memakai server yang sudah Anda punya (rust-analyzer,
  gopls, typescript-language-server, Dart, pyright, clangd…).
- Server mulai saat Anda mengedit file yang dilayaninya atau memakai fitur bahasa, jadi file yang
  hanya dibaca tidak membebani apa pun (Settings → Editor → *Nyalakan server hanya saat
  dibutuhkan*; matikan agar server mulai saat file dibuka). Server berhenti sepuluh menit setelah
  file terakhir ditutup. Server yang belum terpasang disebut di status line beserta perintah
  pemasangannya.

### Dokumen

![Laporan CSV terbuka sebagai spreadsheet di samping terminal](docs/screenshots/docs.png)

Klik file di pohon dan langsung terbuka: halaman **PDF** dirender sesuai zoom, **Word** (`.docx`)
dan **PowerPoint** (`.pptx`) sebagai teks rapi dengan heading, list, tabel, dan gambar, **Excel**
(`.xlsx`, `.xls`, `.ods`) dan **CSV** sebagai grid dengan tab sheet dan header yang tetap terlihat,
serta **gambar**. Semuanya dibaca dengan Rust murni di luar thread UI; hanya yang tampil di layar
yang disimpan di memori.

`⌘F` (`Ctrl+Shift+F`) setelah mengklik dokumen akan mencarinya: kata di halaman PDF disorot tepat di
tempatnya, teks Word dan PowerPoint disorot di tempat, dan di spreadsheet pilihan sel berpindah ke
sel yang cocok, lintas sheet. `Enter` dan `⇧Enter` pindah ke hasil berikutnya dan sebelumnya.

### Review

`⇧⌘R` menampilkan apa yang berubah: sejak agent di pane ini mulai (snapshot diambil di luar index
git, jadi staging area Anda tidak tersentuh), atau sejak commit terakhir. Buka file mana pun dari
diff di editor. Saat agent selesai, tombol **Tinjau** di tile Grid dan di daftar yang menunggu
langsung membukanya. Di worktree, Review menampilkan branch-nya sejak bercabang: commit-nya dan
yang belum di-commit, dengan **Merge** dan **Hapus** di dekatnya.

## Dukungan platform

| | macOS | Windows | Linux |
| --- | :---: | :---: | :---: |
| Project, terminal, tab, split | ✓ | ✓ | ✓ |
| Status agent dari hook, `⌘J`, Grid | ✓ | ✓ | ✓ |
| Notifikasi desktop | ✓ | — | ✓ (`notify-send`) |
| Shell integration | zsh, bash, fish | PowerShell | zsh, bash, fish |
| Title bar dan menu | menu bar sistem | milik TernaryCode, dengan File, Edit, View… | title bar sistem |
| Panel Browser dan tool browser untuk agent | ✓ | browser default | browser default |
| Editor, language server, dokumen, Review, worktree | ✓ | ✓ | ✓ |
| Memperbarui dirinya sendiri | `.dmg`, `.zip`, installer | setup | AppImage, `.tar.gz` |
| Tes end-to-end app sungguhan | ✓ | — | — |

## Shortcut keyboard

Di Windows dan Linux, shortcut `⌘` menjadi `Ctrl+Shift+…` (`⌥` tetap `Alt`), jadi `Ctrl+C`,
`Ctrl+D`, dan sejenisnya tetap sampai ke terminal. Command palette (`⌘K` / `Ctrl+Shift+K`) memuat
semuanya, lengkap dengan shortcut-nya.

| macOS | Windows, Linux | |
| --- | --- | --- |
| `⌘J` · `⇧⌘J` | `Ctrl+Shift+J` · `Ctrl+Alt+J` | agent berikutnya yang menunggu · kembali |
| `⌥⌘J` | `Ctrl+Shift+Alt+J` | daftar yang menunggu |
| `⌘K` | `Ctrl+Shift+K` | command palette |
| `⌘P` | `Ctrl+Shift+P` | pindah project |
| `⇧⌘A` | `Ctrl+Shift+A` | jalankan agent |
| `⌘0` | `Ctrl+Shift+0` | Grid semua project |
| `⌘T` · `⌘W` | `Ctrl+Shift+T` · `Ctrl+Shift+W` | tab baru · tutup pane (atau halaman yang memegang keyboard) |
| `⇧⌘]` · `⇧⌘[` | `Ctrl+Shift+PageDown` · `Ctrl+Shift+PageUp` | tab berikutnya · tab sebelumnya |
| `⌘D` · `⇧⌘D` | `Ctrl+Shift+D` · `Ctrl+Shift+E` | split kanan · split bawah |
| `⌘]` · `⌘[` | `Ctrl+Shift+]` · `Ctrl+Shift+[` | pane berikutnya · pane sebelumnya |
| `⌘O` · `⌘E` · `⌘S` | `Ctrl+Shift+O` · `Ctrl+Shift+Y` · `Ctrl+Shift+S` | buka file · tampilkan editor · simpan |
| `⇧⌘R` | `Ctrl+Shift+R` | review perubahan |
| `⇧⌘B` · `⌥⇧⌘B` | `Ctrl+Shift+Alt+B` | panel Browser · halaman baru (Windows, Linux: server project di browser Anda) |
| `⌘L` · `⌘R` · `⌘[` `⌘]` | | di panel Browser: alamat · muat ulang · mundur, maju (saat halaman memegang keyboard) |
| `⌘F` · `⌘=` `⌘−` | | di panel Browser: cari di halaman · perbesar, perkecil (saat halaman memegang keyboard) |
| `⌘F` · `⇧⌘F` | `Ctrl+Shift+F` · `Ctrl+Shift+G` | cari di pane, file, atau dokumen · di semua pane |
| `⌘B` · `⌘1`–`⌘9` | `Ctrl+Shift+B` · `Ctrl+Shift+1`–`9` | sidebar · project 1–9 |
| `⌘=` · `⌘-` | `Ctrl+Shift+=` · `Ctrl+Shift+-` · `Ctrl+Shift+Backspace` | perbesar · perkecil · ukuran asli teks |
| `⌥⌘K` | `Ctrl+Shift+Alt+K` | bersihkan scrollback |
| `F12` · `⇧F12` · `F2` · `⌘.` | `F12` · `Shift+F12` · `F2` · `Ctrl+.` | definisi · referensi · rename · code action |

Di editor, `Ctrl+Shift+-` adalah Maju (`⌃⇧-` di macOS), seperti di VS Code; di tempat lain
shortcut itu memperkecil teks.

## Kalau ada masalah

**Run Agent bilang agent belum terpasang.** Run Agent mencari program agent-nya (`claude`,
`codex`, `agy`, `opencode`) di `PATH` yang didapat pane baru. **Install in Terminal** mengetik
perintah pemasangannya di terminal project (perintah npm butuh Node.js dulu, dan itu disebutkan);
setelah terpasang, jalankan agent-nya lagi: tab baru langsung menemukannya tanpa restart
TernaryCode. **Run Anyway** menjalankan perintahnya apa adanya, untuk alias atau fungsi shell yang
tidak terlihat oleh pengecekan.

**Windows: `… cannot be loaded because running scripts is disabled on this system`.** Execution
policy bawaan Windows PowerShell memblokir launcher `.ps1` yang dipasang npm (`codex.ps1`,
`npm.ps1`). Jalankan `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` sekali, atau pasang
PowerShell 7 (`winget install Microsoft.PowerShell`), yang otomatis dipakai pane TernaryCode kalau
ada.

**Agent hanya menampilkan *Aktif*, atau status bar bilang belum terhubung.** Agent itu dijalankan
sendiri tanpa hook. Jalankan lewat **Run Agent** (hook untuk sesi itu), atau pasang hook sekali di
**Settings → Integrations**.

**Windows 10: perintah tidak ditandai selesai atau gagal.** Console host Windows 10 bisa membuang
penanda prompt dari shell; status lalu diambil dari hook agent dan program yang sedang berjalan.

**Linux: jendela kosong atau tidak terbuka.** TernaryCode menggambar dengan OpenGL. Pastikan
driver grafis Anda menyediakannya; renderer software Mesa sudah cukup
(`LIBGL_ALWAYS_SOFTWARE=1 ternarycode`).

**Masalah lain?** [Buka issue](https://github.com/ilhamnadhif/ternarycode/issues) dengan versi
TernaryCode, sistem Anda, dan langkah-langkah yang memunculkannya.

## Privasi

Tanpa akun, tanpa telemetri. TernaryCode sendiri hanya terhubung ke GitHub, untuk pemeriksaan
pembaruan, dan itu bisa dimatikan; halaman yang Anda dan agent Anda buka di browser-nya dimuat
seperti di browser mana pun. Hook dan tool browser agent berbicara dengan app lewat socket lokal
privat (named pipe di Windows), dan setiap pane punya token rahasianya sendiri. Login dan cookie
browser tetap di Mac Anda, dalam satu profil yang disimpan WebKit untuk TernaryCode;
**Settings → Browser → Logins & cookies → Clear…** menghapusnya.

Pengaturan dan workspace disimpan di `~/Library/Application Support/ternarycode` (macOS),
`%APPDATA%\ternarycode` (Windows), atau `~/.local/share/ternarycode` (Linux).

## Build dari source

Butuh toolchain Rust stable terbaru (kodenya memakai edition 2024; CI membangun dengan stable).

```sh
cargo build --release      # ternarycode dan ternarycode-hook, berdampingan di target/release
./target/release/ternarycode
```

`ternarycode-hook` harus tetap berada di samping binary app: hook agent menjalankannya, dan ia tidak
melakukan apa-apa di luar pane TernaryCode.

| | |
| --- | --- |
| `scripts/package.sh` | macOS: `TernaryCode.app`, `.dmg`, dan `.zip` (ad-hoc signed kecuali identitas diset) |
| `scripts/package-linux.sh` | Linux: `.tar.gz`, `.deb`, `.rpm`, `.AppImage` |
| `scripts/package-windows.ps1` | Windows: installer Inno Setup dan `.zip` portabel |
| `scripts/e2e.sh` | pengujian end-to-end app sungguhan di macOS (event keyboard dan mouse disuntikkan) |
| `scripts/e2e-web.sh`, `scripts/e2e-web.ps1` | pengujian end-to-end panel Browser di macOS dan Linux, dan di Windows |
| `scripts/release.sh <x.y.z>` | tes, build macOS, tag, lalu CI membangun Windows dan Linux dan mempublikasikan |

### Berkontribusi

Issue dan pull request sangat diterima. Sebelum membuka pull request, jalankan yang dijalankan CI:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/e2e.sh             # macOS: membuka jendela dan mengendalikannya; jangan pakai Mac selama berjalan
```

Kodenya diformat manual, jadi tolong jangan jalankan `cargo fmt` ke seluruh file; ikuti gaya kode
di sekitar perubahan Anda.

### Struktur

| Crate | |
| --- | --- |
| `crates/tc-term` | inti terminal (alacritty_terminal), PTY (Unix dan ConPTY), sinyal OSC, pencarian, widget terminal egui |
| `crates/tc-core` | model, preset dan pengenalan agent, state machine status, antrean perhatian, file atomik |
| `crates/tc-agent` | info proses, protokol dan server hook, `ternarycode-hook` (dan tool browser-nya, `ternarycode-hook mcp`), integrasi agent, shell integration |
| `crates/tc-git` | snapshot dan diff lewat git CLI, tanpa menyentuh index |
| `crates/tc-lsp` | klien language server: transport, registry server, posisi, edit |
| `crates/tc-doc` | pembaca PDF, Word, PowerPoint, spreadsheet, CSV, dan gambar |
| `crates/tc-web` | `ternarycode-web`, program pembantu halaman di Linux (halaman WebKitGTK di jendela X11 app) |
| `src/` | app-nya (egui), dan browser-nya (wry: WebKit, WebView2; WebKitGTK lewat `tc-web`) |

## Dibangun dengan

[egui dan eframe](https://github.com/emilk/egui) (UI) di atas [wgpu](https://github.com/gfx-rs/wgpu)
dan OpenGL, [alacritty_terminal](https://github.com/alacritty/alacritty) (emulasi terminal),
[wry](https://github.com/tauri-apps/wry) (browser, di atas WebKit sistem),
[hayro](https://github.com/LaurenzV/hayro) (PDF), [calamine](https://github.com/tafia/calamine)
(spreadsheet), [quick-xml](https://github.com/tafia/quick-xml) dan
[zip](https://github.com/zip-rs/zip2) (Word dan PowerPoint),
[lsp-types](https://github.com/gluon-lang/lsp-types), dan [notify](https://github.com/notify-rs/notify).
Font: [Inter](https://rsms.me/inter/) dan ikon [Phosphor](https://phosphoricons.com).

## Keluarga Ternary

TernaryCode adalah satu dari empat app Rust native kecil yang berbagi tampilan dan ikon kisi 3 × 3:
[TernaryGrid](https://github.com/ilhamnadhif/ternarygrid) (klien SQL),
[TernaryWire](https://github.com/ilhamnadhif/ternarywire) (klien API),
[TernaryShell](https://github.com/ilhamnadhif/ternaryshell) (klien SSH dan SFTP), dan TernaryCode.

## Lisensi

MIT. Font: Inter (SIL OFL 1.1) dan ikon Phosphor (MIT).
