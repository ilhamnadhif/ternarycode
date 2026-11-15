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
  <a href="README.md">English</a> ·
  <a href="#instalasi">Instalasi</a> ·
  <a href="#fitur">Fitur</a> ·
  <a href="#shortcut-keyboard">Shortcut</a> ·
  <a href="#build-dari-source">Build</a>
</p>

![TernaryCode: lima project di sidebar, agent meminta izin di terminal, editor di sampingnya](docs/screenshots/focus.png)

Menjalankan Claude Code, Codex, Gemini CLI, atau OpenCode di lima project sekaligus biasanya
berarti lima jendela editor, masing-masing dengan agent tersembunyi di panel terminal, tanpa tahu
mana yang sedang menunggu Anda. TernaryCode menaruh semua project di **satu jendela**: masing-masing
dengan terminal, tab, dan split pane sendiri, **Grid** yang menampilkan semuanya sekaligus, dan
satu pertanyaan yang dijawab cepat dan jujur: **agent mana yang butuh saya sekarang?**

Aplikasinya satu binary native kecil (tanpa Electron atau Node.js; browser-nya memakai WebKit
bawaan macOS): jendela muncul sekitar 0,2 detik, CPU 0 % saat diam, dan sepuluh pane terbuka
memakai sekitar 70 MB.

## Sorotan

- **Status agent yang bisa dipercaya.** Setiap pane tahu apakah agent-nya sedang bekerja, menunggu
  Anda, selesai, atau gagal, dari hook agent itu sendiri, notifikasi terminal, dan shell. Setiap
  status menyebut sumbernya; tanpa sinyal yang bisa dipercaya, ia hanya menyebut agent-nya
  *Aktif*, bukan menebak.
- **Lompat ke yang butuh Anda.** `⌘J` membuka agent paling mendesak yang menunggu, `⇧⌘J`
  kembali. Status bar menyebut namanya, Dock menampilkan badge, dan notifikasi muncul saat jendela
  di latar belakang. Fokus keyboard tidak pernah pindah sendiri.
- **Grid** menampilkan semua project secara live, berdampingan, dengan terminal yang memenuhi
  kartunya: switch **Focus | Grid** di bagian atas sidebar (atau `⌘0`) membukanya, dan project di
  daftar membuka project itu sendiri. **Aktif** di tengah header-nya hanya menampilkan project yang
  sedang menjalankan sesuatu atau menunggu. Project dengan beberapa tab menampilkan tab yang butuh
  Anda (atau tempat agent bekerja), tanpa pernah berpindah dari tab yang sedang Anda ketik; tab-tabnya
  ada di header tile, untuk dipilih sendiri.
- **Browser di samping terminal** (macOS). Halaman project terbuka di **panel Browser**-nya, dan
  agent memakainya lewat tool browser TernaryCode sendiri untuk memeriksa hasil kerjanya, masing-masing
  di halamannya sendiri: Anda melihat setiap kliknya, dan bisa mengambil alih kapan saja.
- **Editor di samping terminal**, dengan language server yang sudah Anda punya (rust-analyzer,
  gopls, typescript-language-server, Dart, pyright, clangd…): ⌘-klik ke definisi, referensi,
  rename lintas file, code action, autocomplete, hover, dan daftar problem.
- **Dokumen langsung terbuka**: PDF, Word, PowerPoint, Excel, CSV, dan gambar, hanya-baca dan cepat,
  dan bisa dicari dengan `⌘F`.
- **`ternarycode nama-project`** di terminal mana pun membuka project (atau folder) itu di jendela.
- **Review** apa yang diubah agent sejak ia mulai, atau sejak commit terakhir.
- **Memperbarui dirinya sendiri** dari rilis GitHub, cukup satu klik.

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
seperti di terminal lain (Claude Code, Codex, Gemini CLI, dan OpenCode berjalan native di Windows),
dan hook-nya sampai ke app lewat named pipe. Di Windows 10, console host bisa membuang penanda
prompt dari shell; status lalu diambil dari hook agent dan program yang sedang berjalan.

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
tangan kode), lalu memulai ulang ke versi baru dengan project dan tab Anda dipulihkan. Pemeriksaan
berikutnya tidak mengganggu: muncul item **Update** di status bar.

App macOS, setup Windows, AppImage, dan `.tar.gz` memperbarui dirinya sendiri; `.deb`, `.rpm`, dan
`.zip` portabel diperbarui dengan memasang paket baru. Matikan pemeriksaan otomatis di
**Settings → General**, atau pakai **Check for Updates…** kapan saja. Selain halaman yang Anda dan
agent Anda buka di browser-nya, pemeriksaan pembaruan adalah satu-satunya koneksi jaringan yang
dibuat TernaryCode.

## Fitur

### Project, terminal, dan agent

- **Project adalah folder.** Terminalnya mulai di sana, file dan perubahan git-nya tampil di
  sampingnya, dan sidebar menunjukkan jenisnya (Rust, Go, TypeScript, React, Flutter, Python…),
  branch, jumlah perubahan, dan apa yang sedang dikerjakan agent-nya. Project diurutkan menurut
  nama, di sidebar maupun di Grid.
- **Tab dan split pane per project**, bisa diseret untuk diatur ulang, dan dipulihkan saat app
  dibuka lagi. Shell yang dipulihkan baru berjalan saat Anda melihatnya, jadi workspace berisi dua
  puluh project terbuka secepat satu project.
- **Jalankan agent** dari preset (Claude Code, Codex, Gemini CLI, OpenCode, atau perintah Anda
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
  dan yang ada di program yang memakai mouse. Hyperlink OSC 8 juga bisa
  dibuka. `path:baris` terbuka di editor. Klik kanan link untuk membuka atau menyalinnya.
- **Drop file ke terminal** dan path-nya langsung terketik di sana, jadi agent bisa membaca
  screenshot. Folder yang di-drop ke sidebar menjadi project. **`⌘V` dengan gambar** di
  clipboard diteruskan ke Claude Code, Codex, Gemini CLI, atau OpenCode.
- **Server lokal langsung kelihatan.** Kalau pane mencetak `http://localhost:5173` (Vite,
  Next.js, Rails, Django…), alamatnya muncul di header project dan tile Grid selama servernya
  menjawab. Klik untuk membuka halamannya di panel Browser.
- **Shell integration** (zsh, bash, fish, dan PowerShell) menandai prompt dan perintah, jadi perintah yang gagal
  atau selesai setelah lama tetap ketahuan, dan resize tidak pernah meninggalkan potongan prompt.

### Tahu agent mana yang butuh Anda

![Grid: empat project berdampingan, dua agent menunggu izin](docs/screenshots/grid.png)

- Status tiap pane berasal dari, urut dari yang paling dipercaya: **hook** agent (dipasang per
  agent di Settings → Integrations, atau ditambahkan per peluncuran tanpa mengubah config Anda),
  notifikasi terminal (OSC 9, 99, dan 777), shell integration (OSC 133), dan proses foreground.
- **Butuh input**, **Berjalan**, **Selesai**, **Gagal**, dan **Diam** tampil di sidebar, tab, kartu
  Grid, dan status bar; daftar perhatian (`⌥⌘J`) menampilkan semua yang menunggu, yang paling lama
  lebih dulu.
- Notifikasi desktop dan badge Dock hanya muncul kalau Anda tidak sedang melihatnya.

| Agent | Per peluncuran (tanpa mengubah file) | Dipasang sekali |
| --- | --- | --- |
| Claude Code | `claude --mcp-config "$TC_CLAUDE_MCP" --settings "$TC_CLAUDE_SETTINGS"` menambah hook dan tool browser untuk sesi itu | hook di `~/.claude/settings.json` |
| Codex | `codex -c "$TC_CODEX_HOOKS" -c "$TC_CODEX_NOTIFY" -c "$TC_CODEX_MCP"` | blok terkelola di `~/.codex/config.toml`, termasuk tool browser |
| Gemini CLI | — | hook dan tool browser di `~/.gemini/settings.json` |
| OpenCode | — | plugin `~/.config/opencode/plugins/ternarycode.js` |

Pemasangan menampilkan perubahan persisnya lebih dulu dan menyimpan backup; **Uninstall**
mengembalikan file byte demi byte kalau Anda belum mengubahnya sejak itu. Tool browser ada di macOS
(`--mcp-config` dan `$TC_CODEX_MCP` hanya ditambahkan di sana); Settings → Browser menunjukkan agent
mana yang sudah memilikinya.

### Browser untuk Anda dan agent Anda

Di macOS setiap project punya **panel Browser** di kanan terminalnya, di tempat laci Code (keduanya
bergantian di sana). Di Windows dan Linux, halaman untuk sementara terbuka di browser default Anda.

- **Halaman terbuka di sana**: globe di top bar atau `⇧⌘B` menampilkan dan menyembunyikan panel,
  `⌘L` mengetik alamat, `⌥⇧⌘B` membuka halaman baru, `⌘R` memuat ulang, dan `⌘[` `⌘]` mundur dan
  maju. Chip server, link yang di-`⌘`-klik, **Open in Browser** pada file di Files,
  `ternarycode browse <url>`, dan program yang membuka browser dari pane (`$BROWSER`,
  `open https://…`, `vite --open`) semuanya membuka halamannya di sana. Tahan `⌥` untuk browser
  default Anda.
- **Agent memeriksa hasil kerjanya sendiri di sana.** Agent yang dijalankan dari TernaryCode
  mendapat tool browser-nya (membuka halaman, klik, mengetik, memilih opsi, menekan tombol, scroll,
  menunggu, membaca halaman, screenshot, membaca console). Minta agent memeriksa perubahannya di
  browser dan halamannya terbuka di sampingnya, berbingkai biru, dengan bar yang menyebut apa yang
  sedang ia lakukan (“Clicking the Add to cart button”). Klik atau ketik di halaman untuk
  **Take Over**; **Hand Back** membiarkan agent melanjutkan.
- **Satu halaman per agent.** Dua agent di satu tab masing-masing mendapat halaman sendiri,
  bertanda CC, Cx… di tab-nya. Halaman yang di depan mengikuti terminal yang Anda klik; agent yang
  bekerja di halaman di belakangnya memberi titik biru di sana. Link yang Anda buka tidak pernah
  mengambil halaman agent.
- **Terminal tidak pernah diperas.** Terminal berdampingan yang jadi terlalu sempit di samping panel
  ditampilkan bertumpuk selama panel terbuka (**Keep Side by Side** kalau Anda lebih suka tetap
  berdampingan); kalau tetap tidak muat, panel mengambang di atasnya sampai Anda mengklik terminal.
  `⤢` membuat halaman memenuhi jendela.
- **Agent tetap di jalur aman.** Agent membuka alamat mesin ini (localhost dan sejenisnya) dan situs
  yang Anda izinkan, untuk semua project atau satu project; situs lain menunggu **Allow Once**,
  **Always**, atau **Block**, ditanyakan di TernaryCode bersama pertanyaan lain yang menunggu Anda.
  JavaScript milik agent sendiri (`browser_evaluate`) mati sampai Anda mengizinkannya di
  Settings → Browser.
- **Di Grid**, tile project menampilkan panelnya di samping terminal, halamannya diperkecil supaya
  tata letaknya sama dengan di Focus, dan tetap hidup; tile kecil menampilkan gambar halaman yang
  sedang dipakai agent.
- Satu profil login untuk semua project, tetap tersimpan melewati restart dan pembaruan. Warning dan
  error di console halaman tampil di toolbar-nya. Halaman yang tidak terlihat dan tidak dipakai
  tidur setelah 10 menit (tidak pernah selama agent memakainya), dan status bar menampilkan memori
  yang dipakai halaman.

### Editor dan fitur bahasa

- Tab, nomor baris, warna sintaks, pencarian, dan penyimpanan aman: tidak pernah menimpa file yang
  diubah agent saat Anda punya perubahan yang belum disimpan (Anda yang memilih).
- Dengan language server terpasang: go to definition (`F12` atau `⌘`-klik), type definition,
  implementation, referensi (`⇧F12`), rename lintas file (`F2`, bisa di-undo sekali), code action
  dan quick fix (`⌘.`), autocomplete, hover, signature help, simbol dokumen dan workspace, problem
  (`⇧⌘M`), dan format saat menyimpan.
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
| `⌘F` · `⇧⌘F` | `Ctrl+Shift+F` · `Ctrl+Shift+G` | cari di pane, file, atau dokumen · di semua pane |
| `⌘B` · `⌘1`–`⌘9` | `Ctrl+Shift+B` · `Ctrl+Shift+1`–`9` | sidebar · project 1–9 |
| `⌘=` · `⌘-` | `Ctrl+Shift+=` · `Ctrl+Shift+-` · `Ctrl+Shift+Backspace` | perbesar · perkecil · ukuran asli teks |
| `⌥⌘K` | `Ctrl+Shift+Alt+K` | bersihkan scrollback |
| `F12` · `⇧F12` · `F2` · `⌘.` | `F12` · `Shift+F12` · `F2` · `Ctrl+.` | definisi · referensi · rename · code action |

Di editor, `Ctrl+Shift+-` adalah Maju (`⌃⇧-` di macOS), seperti di VS Code; di tempat lain
shortcut itu memperkecil teks.

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

Butuh toolchain Rust stable yang baru (dibangun dengan 1.99).

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
| `scripts/release.sh <x.y.z>` | tes, build macOS, tag, lalu CI membangun Windows dan Linux dan mempublikasikan |

## Keluarga Ternary

TernaryCode adalah satu dari empat app Rust native kecil yang berbagi tampilan dan ikon kisi 3 × 3:
[TernaryGrid](https://github.com/ilhamnadhif/ternarygrid) (klien SQL),
[TernaryWire](https://github.com/ilhamnadhif/ternarywire) (klien API),
[TernaryShell](https://github.com/ilhamnadhif/ternaryshell) (klien SSH dan SFTP), dan TernaryCode.

## Lisensi

MIT. Font: Inter (SIL OFL 1.1) dan ikon Phosphor (MIT).
