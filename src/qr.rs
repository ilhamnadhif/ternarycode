//! Pure-Rust ISO/IEC 18004 QR Code encoder (Byte mode, Error Correction Level M, Versions 1–10)
//! for Phone Pairing. Supports pairing URLs up to 213 UTF-8 bytes with zero external crates.

/// A generated QR code matrix (`size × size` modules; `true` = dark).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCode {
    pub version: u8,
    pub size: usize,
    modules: Vec<bool>,
}

struct VersionSpec {
    version: u8,
    total_codewords: usize,
    data_codewords: usize,
    ec_per_block: usize,
    g1_blocks: usize,
    g1_data: usize,
    g2_blocks: usize,
    g2_data: usize,
    alignments: &'static [usize],
}

/// ISO/IEC 18004 Table 9 (EC Level M) and Annex E (alignment centers), Versions 1–10.
const SPECS: [VersionSpec; 10] = [
    VersionSpec { version: 1, total_codewords: 26, data_codewords: 16, ec_per_block: 10, g1_blocks: 1, g1_data: 16, g2_blocks: 0, g2_data: 0, alignments: &[] },
    VersionSpec { version: 2, total_codewords: 44, data_codewords: 28, ec_per_block: 16, g1_blocks: 1, g1_data: 28, g2_blocks: 0, g2_data: 0, alignments: &[6, 18] },
    VersionSpec { version: 3, total_codewords: 70, data_codewords: 44, ec_per_block: 26, g1_blocks: 1, g1_data: 44, g2_blocks: 0, g2_data: 0, alignments: &[6, 22] },
    VersionSpec { version: 4, total_codewords: 100, data_codewords: 64, ec_per_block: 18, g1_blocks: 2, g1_data: 32, g2_blocks: 0, g2_data: 0, alignments: &[6, 26] },
    VersionSpec { version: 5, total_codewords: 134, data_codewords: 86, ec_per_block: 24, g1_blocks: 2, g1_data: 43, g2_blocks: 0, g2_data: 0, alignments: &[6, 30] },
    VersionSpec { version: 6, total_codewords: 172, data_codewords: 108, ec_per_block: 16, g1_blocks: 4, g1_data: 27, g2_blocks: 0, g2_data: 0, alignments: &[6, 34] },
    VersionSpec { version: 7, total_codewords: 196, data_codewords: 124, ec_per_block: 18, g1_blocks: 4, g1_data: 31, g2_blocks: 0, g2_data: 0, alignments: &[6, 22, 38] },
    VersionSpec { version: 8, total_codewords: 242, data_codewords: 154, ec_per_block: 22, g1_blocks: 2, g1_data: 38, g2_blocks: 2, g2_data: 39, alignments: &[6, 24, 42] },
    VersionSpec { version: 9, total_codewords: 292, data_codewords: 182, ec_per_block: 22, g1_blocks: 3, g1_data: 36, g2_blocks: 2, g2_data: 37, alignments: &[6, 26, 46] },
    VersionSpec { version: 10, total_codewords: 346, data_codewords: 216, ec_per_block: 26, g1_blocks: 4, g1_data: 43, g2_blocks: 1, g2_data: 44, alignments: &[6, 28, 50] },
];

impl QrCode {
    /// Encodes `data` in Byte mode at Error Correction Level M. Returns `None` if `data` exceeds
    /// Version 10 capacity (213 bytes).
    pub fn encode(data: &[u8]) -> Option<Self> {
        let spec = SPECS.iter().find(|s| {
            let count_bits = if s.version <= 9 { 8 } else { 16 };
            let header_bits = 4 + count_bits;
            header_bits + data.len() * 8 <= s.data_codewords * 8
        })?;

        let data_cw = encode_data_codewords(data, spec);
        let final_cw = interleave_with_ec(&data_cw, spec);

        let size = (spec.version as usize) * 4 + 17;
        let mut base = vec![false; size * size];
        let mut function = vec![false; size * size];

        draw_function_patterns(&mut base, &mut function, size, spec);
        draw_codewords(&mut base, &function, size, &final_cw);

        let mut best_mask = 0u8;
        let mut best_penalty = u32::MAX;
        let mut best_modules = Vec::new();
        for mask in 0..8u8 {
            let mut trial = base.clone();
            apply_mask(&mut trial, &function, size, mask);
            draw_format_bits(&mut trial, size, mask);
            let penalty = penalty_score(&trial, size);
            if penalty < best_penalty {
                best_penalty = penalty;
                best_mask = mask;
                best_modules = trial;
            }
        }
        let _ = best_mask;

        Some(Self {
            version: spec.version,
            size,
            modules: best_modules,
        })
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        if x < self.size && y < self.size {
            self.modules[y * self.size + x]
        } else {
            false
        }
    }
}

fn push_bits(bits: &mut Vec<bool>, val: u32, len: usize) {
    for i in (0..len).rev() {
        bits.push(((val >> i) & 1) != 0);
    }
}

fn encode_data_codewords(data: &[u8], spec: &VersionSpec) -> Vec<u8> {
    let cap_bits = spec.data_codewords * 8;
    let mut bits = Vec::with_capacity(cap_bits);
    // Byte mode indicator: 0100.
    push_bits(&mut bits, 0b0100, 4);
    let count_bits = if spec.version <= 9 { 8 } else { 16 };
    push_bits(&mut bits, data.len() as u32, count_bits);
    for &b in data {
        push_bits(&mut bits, b as u32, 8);
    }
    // Terminator (up to 4 zero bits).
    let term = (cap_bits - bits.len()).min(4);
    push_bits(&mut bits, 0, term);
    // Pad to byte boundary.
    while !bits.len().is_multiple_of(8) {
        bits.push(false);
    }
    let mut out = Vec::with_capacity(spec.data_codewords);
    for chunk in bits.as_chunks::<8>().0 {
        let mut byte = 0u8;
        for &bit in chunk {
            byte = (byte << 1) | u8::from(bit);
        }
        out.push(byte);
    }
    // Alternating pad bytes 0xEC, 0x11.
    let mut toggle = false;
    while out.len() < spec.data_codewords {
        out.push(if toggle { 0x11 } else { 0xEC });
        toggle = !toggle;
    }
    out
}

/// GF(256) multiplication modulo x^8 + x^4 + x^3 + x^2 + 1 (`0x11D`).
fn gf_mul(mut x: u8, mut y: u8) -> u8 {
    let mut z = 0u8;
    for _ in 0..8 {
        if (y & 1) != 0 {
            z ^= x;
        }
        let carry = (x & 0x80) != 0;
        x <<= 1;
        if carry {
            x ^= 0x1D;
        }
        y >>= 1;
    }
    z
}

/// Reed-Solomon generator polynomial coefficients (excluding leading 1) for `degree` EC bytes.
fn rs_generator(degree: usize) -> Vec<u8> {
    let mut coeffs = vec![0u8; degree];
    coeffs[degree - 1] = 1;
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            coeffs[j] = gf_mul(coeffs[j], root);
            if j + 1 < degree {
                coeffs[j] ^= coeffs[j + 1];
            }
        }
        root = gf_mul(root, 0x02);
    }
    coeffs
}

fn rs_remainder(data: &[u8], poly: &[u8]) -> Vec<u8> {
    let mut rem = vec![0u8; poly.len()];
    for &b in data {
        let factor = b ^ rem[0];
        rem.rotate_left(1);
        *rem.last_mut().unwrap() = 0;
        for (r, &g) in rem.iter_mut().zip(poly.iter()) {
            *r ^= gf_mul(g, factor);
        }
    }
    rem
}

fn interleave_with_ec(data: &[u8], spec: &VersionSpec) -> Vec<u8> {
    let poly = rs_generator(spec.ec_per_block);
    let mut data_blocks: Vec<&[u8]> = Vec::with_capacity(spec.g1_blocks + spec.g2_blocks);
    let mut ec_blocks: Vec<Vec<u8>> = Vec::with_capacity(spec.g1_blocks + spec.g2_blocks);
    let mut offset = 0;
    for _ in 0..spec.g1_blocks {
        let slice = &data[offset..offset + spec.g1_data];
        offset += spec.g1_data;
        ec_blocks.push(rs_remainder(slice, &poly));
        data_blocks.push(slice);
    }
    for _ in 0..spec.g2_blocks {
        let slice = &data[offset..offset + spec.g2_data];
        offset += spec.g2_data;
        ec_blocks.push(rs_remainder(slice, &poly));
        data_blocks.push(slice);
    }

    let mut out = Vec::with_capacity(spec.total_codewords);
    let max_data = spec.g1_data.max(spec.g2_data);
    for i in 0..max_data {
        for block in &data_blocks {
            if let Some(&b) = block.get(i) {
                out.push(b);
            }
        }
    }
    for i in 0..spec.ec_per_block {
        for block in &ec_blocks {
            out.push(block[i]);
        }
    }
    out
}

fn set_func(modules: &mut [bool], function: &mut [bool], size: usize, x: isize, y: isize, dark: bool) {
    if x >= 0 && y >= 0 && (x as usize) < size && (y as usize) < size {
        let idx = (y as usize) * size + (x as usize);
        modules[idx] = dark;
        function[idx] = true;
    }
}

fn draw_finder(modules: &mut [bool], function: &mut [bool], size: usize, cx: isize, cy: isize) {
    for dy in -4isize..=4 {
        for dx in -4isize..=4 {
            let d = dx.abs().max(dy.abs());
            let dark = d != 2 && d != 4;
            set_func(modules, function, size, cx + dx, cy + dy, dark);
        }
    }
}

fn draw_alignment(modules: &mut [bool], function: &mut [bool], size: usize, cx: isize, cy: isize) {
    for dy in -2isize..=2 {
        for dx in -2isize..=2 {
            let dark = dx.abs().max(dy.abs()) != 1;
            set_func(modules, function, size, cx + dx, cy + dy, dark);
        }
    }
}

fn draw_function_patterns(modules: &mut [bool], function: &mut [bool], size: usize, spec: &VersionSpec) {
    // Timing patterns.
    for i in 0..size {
        let dark = i % 2 == 0;
        set_func(modules, function, size, 6, i as isize, dark);
        set_func(modules, function, size, i as isize, 6, dark);
    }
    // Finder patterns + separators.
    let s = size as isize;
    draw_finder(modules, function, size, 3, 3);
    draw_finder(modules, function, size, s - 4, 3);
    draw_finder(modules, function, size, 3, s - 4);

    // Alignment patterns (skipping the three finder corners).
    let centers = spec.alignments;
    let n = centers.len();
    for (i, &cy) in centers.iter().enumerate() {
        for (j, &cx) in centers.iter().enumerate() {
            if (i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0) {
                continue;
            }
            draw_alignment(modules, function, size, cx as isize, cy as isize);
        }
    }

    // Reserve format information areas (actual bits written per mask).
    for i in 0..=8 {
        if i != 6 {
            set_func(modules, function, size, 8, i as isize, false);
            set_func(modules, function, size, i as isize, 8, false);
        }
    }
    for i in 0..8 {
        set_func(modules, function, size, (size - 1 - i) as isize, 8, false);
        if i < 7 {
            set_func(modules, function, size, 8, (size - 1 - i) as isize, false);
        }
    }
    // Dark module.
    set_func(modules, function, size, 8, (size - 8) as isize, true);

    // Version information blocks for Version 7+.
    if spec.version >= 7 {
        let bits = version_info_bits(spec.version);
        for i in 0..18 {
            let dark = ((bits >> i) & 1) != 0;
            let a = (size - 11 + (i % 3)) as isize;
            let b = (i / 3) as isize;
            set_func(modules, function, size, a, b, dark);
            set_func(modules, function, size, b, a, dark);
        }
    }
}

fn version_info_bits(version: u8) -> u32 {
    let mut rem = (version as u32) << 12;
    for i in (0..6).rev() {
        if ((rem >> (i + 12)) & 1) != 0 {
            rem ^= 0x1F25 << i;
        }
    }
    ((version as u32) << 12) | rem
}

fn format_info_bits(mask: u8) -> u16 {
    // EC Level M indicator = 00.
    let data = u16::from(mask & 7);
    let mut rem = data << 10;
    for i in (0..5).rev() {
        if ((rem >> (i + 10)) & 1) != 0 {
            rem ^= 0x537 << i;
        }
    }
    ((data << 10) | rem) ^ 0x5412
}

fn draw_format_bits(modules: &mut [bool], size: usize, mask: u8) {
    let bits = format_info_bits(mask);
    let bit = |i: usize| ((bits >> i) & 1) != 0;
    // Around top-left finder.
    for i in 0..=5 {
        modules[i * size + 8] = bit(i);
    }
    modules[7 * size + 8] = bit(6);
    modules[8 * size + 8] = bit(7);
    modules[8 * size + 7] = bit(8);
    for i in 9..15 {
        modules[8 * size + (14 - i)] = bit(i);
    }
    // Split between top-right and bottom-left finders.
    for i in 0..8 {
        modules[8 * size + (size - 1 - i)] = bit(i);
    }
    for i in 8..15 {
        modules[(size - 15 + i) * size + 8] = bit(i);
    }
    modules[(size - 8) * size + 8] = true;
}

fn draw_codewords(modules: &mut [bool], function: &[bool], size: usize, codewords: &[u8]) {
    let mut bit_idx = 0usize;
    let total_bits = codewords.len() * 8;
    let mut right = (size as isize) - 1;
    let mut upward = true;
    while right >= 1 {
        if right == 6 {
            right -= 1;
        }
        for step in 0..size {
            let y = if upward { size - 1 - step } else { step };
            for dx in 0..2 {
                let x = (right - dx) as usize;
                let idx = y * size + x;
                if !function[idx] {
                    let dark = if bit_idx < total_bits {
                        ((codewords[bit_idx / 8] >> (7 - (bit_idx % 8))) & 1) != 0
                    } else {
                        false
                    };
                    modules[idx] = dark;
                    bit_idx += 1;
                }
            }
        }
        upward = !upward;
        right -= 2;
    }
}

fn mask_bit(mask: u8, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y).is_multiple_of(2),
        1 => y.is_multiple_of(2),
        2 => x.is_multiple_of(3),
        3 => (x + y).is_multiple_of(3),
        4 => ((y / 2) + (x / 3)).is_multiple_of(2),
        5 => ((x * y) % 2) + ((x * y) % 3) == 0,
        6 => (((x * y) % 2) + ((x * y) % 3)).is_multiple_of(2),
        _ => (((x + y) % 2) + ((x * y) % 3)).is_multiple_of(2),
    }
}

fn apply_mask(modules: &mut [bool], function: &[bool], size: usize, mask: u8) {
    for y in 0..size {
        for x in 0..size {
            let idx = y * size + x;
            if !function[idx] && mask_bit(mask, x, y) {
                modules[idx] = !modules[idx];
            }
        }
    }
}

fn penalty_score(modules: &[bool], size: usize) -> u32 {
    let at = |x: usize, y: usize| modules[y * size + x];
    let mut score = 0u32;
    // Rule 1: adjacent modules in same color (rows & columns).
    for y in 0..size {
        let mut run_color = at(0, y);
        let mut run_len = 1u32;
        for x in 1..size {
            let c = at(x, y);
            if c == run_color {
                run_len += 1;
                if run_len == 5 {
                    score += 3;
                } else if run_len > 5 {
                    score += 1;
                }
            } else {
                run_color = c;
                run_len = 1;
            }
        }
    }
    for x in 0..size {
        let mut run_color = at(x, 0);
        let mut run_len = 1u32;
        for y in 1..size {
            let c = at(x, y);
            if c == run_color {
                run_len += 1;
                if run_len == 5 {
                    score += 3;
                } else if run_len > 5 {
                    score += 1;
                }
            } else {
                run_color = c;
                run_len = 1;
            }
        }
    }
    // Rule 2: 2x2 blocks of same color.
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let c = at(x, y);
            if c == at(x + 1, y) && c == at(x, y + 1) && c == at(x + 1, y + 1) {
                score += 3;
            }
        }
    }
    // Rule 3: finder-like 1:1:3:1:1 patterns.
    let pat1 = [true, false, true, true, true, false, true, false, false, false, false];
    let pat2 = [false, false, false, false, true, false, true, true, true, false, true];
    for y in 0..size {
        for x in 0..=size.saturating_sub(11) {
            if (0..11).all(|k| at(x + k, y) == pat1[k]) || (0..11).all(|k| at(x + k, y) == pat2[k]) {
                score += 40;
            }
        }
    }
    for x in 0..size {
        for y in 0..=size.saturating_sub(11) {
            if (0..11).all(|k| at(x, y + k) == pat1[k]) || (0..11).all(|k| at(x, y + k) == pat2[k]) {
                score += 40;
            }
        }
    }
    // Rule 4: dark/light balance.
    let dark = modules.iter().filter(|&&m| m).count() as i32;
    let total = (size * size) as i32;
    let k = ((dark * 20 - total * 10).abs() + total - 1) / total - 1;
    score + (k.max(0) as u32) * 10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_version_bch_match_iso_vectors() {
        // ISO/IEC 18004 Annex C: EC Level M, Mask 0 -> 101010000010010 (0x5412).
        assert_eq!(format_info_bits(0), 0x5412);
        // EC Level M, Mask 1 -> 101000100100101 (0x5125).
        assert_eq!(format_info_bits(1), 0x5125);
        // ISO/IEC 18004 Annex D: Version 7 -> 000111110010010100 (0x07C94).
        assert_eq!(version_info_bits(7), 0x07C94);
    }

    #[test]
    fn encodes_pairing_urls_across_versions() {
        let short = QrCode::encode(b"http://192.168.1.5:7890/?pair=ABCD1234").unwrap();
        assert_eq!(short.version, 3);
        assert_eq!(short.size, 29);
        // Top-left finder center 3x3 is dark; ring around it is light.
        for y in 2..=4 {
            for x in 2..=4 {
                assert!(short.get(x, y));
            }
        }
        assert!(!short.get(1, 1));
        assert!(short.get(0, 0));
        // Dark module always set at (8, size - 8).
        assert!(short.get(8, short.size - 8));

        // Longer tunnel URL uses Version 7+ (includes version info bits).
        let long_url = format!("https://ternarycode-companion-tunnel.trycloudflare.com/?pair={}", "a".repeat(80));
        let long = QrCode::encode(long_url.as_bytes()).unwrap();
        assert!(long.version >= 7);
        assert_eq!(long.size, long.version as usize * 4 + 17);
        assert!(long.get(8, long.size - 8));

        // Exceeding Version 10 capacity returns None cleanly.
        assert!(QrCode::encode(&vec![b'x'; 250]).is_none());
    }
}
