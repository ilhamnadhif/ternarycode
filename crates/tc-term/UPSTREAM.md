# Asal kode tc-term

`tc-term` berisi salinan inti terminal TernaryShell yang berkembang sendiri (keputusan 9 Nov 2026,
PRD bagian 16). TernaryShell tidak diubah.

- Repo asal: `~/Developments/RUST/ternaryshell` (github.com/ilhamnadhif/ternaryshell)
- Commit asal: `d944572fa882923d96271c989e66d7bcb8928bff` (Release 1.4.5)
- Disalin pada: 9 November 2026

| File di tc-term | Asal di TernaryShell |
| --- | --- |
| `src/core.rs` | `src/term/core.rs` |
| `src/input.rs` | `src/term/input.rs` |
| `src/scheme.rs` | `src/term/scheme.rs` |
| `src/widget.rs` | `src/ui/terminal.rs` |
| `src/layout.rs` | `src/ui/layout.rs` |
| `src/fonts.rs` | `src/ui/fonts.rs` |
| `assets/fonts/` | `assets/fonts/` |

## Perubahan terhadap asal

Perilaku yang sengaja berbeda dari TernaryShell; jangan ditimpa saat port.

- Protokol keyboard kitty (`src/input.rs`, `src/widget.rs`, `src/core.rs`): `term_config` dan
  `new_terminal` selalu memasang `kitty_keyboard: true` (alacritty mengosongkan stack-nya setiap
  kali opsi itu berubah lewat `set_options`, jadi setiap `Config` harus memilikinya). Tombol
  di-encode oleh `input::encode_event` sesuai flag yang dipasang program; tanpa flag tetap
  `encode_key` legacy yang lama. Di widget, tombol menunggu event berikutnya: teks yang diketiknya
  ikut di kode escape-nya dan tidak diketik lagi. Ctrl+C/X/V yang dijadikan event clipboard oleh
  egui (Windows, Linux) ikut di-encode.
- `end_session` mengosongkan stack mode keyboard kedua layar (`clear_keyboard_modes`), supaya
  program berikutnya di pane itu mulai dengan tombol legacy.
- `Feeder` menghitung push `CSI > flags u` (`KeyboardPushes`) dan mengosongkan stack setiap 4000
  push: alacritty 0.26 panic pada push ke-4097 (ia mengambil dari `title_stack`, bukan dari stack
  keyboard). Hapus bila versi alacritty yang dipakai sudah memperbaikinya.

## Cara port perbaikan

Setiap rilis, jalankan:

```sh
git -C ~/Developments/RUST/ternaryshell log --oneline <commit-asal>..HEAD -- src/term src/ui/terminal.rs src/ui/layout.rs src/ui/fonts.rs
```

lalu port perbaikan bug yang relevan secara manual dan perbarui commit asal di atas.
