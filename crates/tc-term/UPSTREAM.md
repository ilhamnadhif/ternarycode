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

## Cara port perbaikan

Setiap rilis, jalankan:

```sh
git -C ~/Developments/RUST/ternaryshell log --oneline <commit-asal>..HEAD -- src/term src/ui/terminal.rs src/ui/layout.rs src/ui/fonts.rs
```

lalu port perbaikan bug yang relevan secara manual dan perbarui commit asal di atas.
