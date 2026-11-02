//! A small PNG writer with real deflate (LZ77 plus the fixed Huffman code), so a rendered
//! portrait fits the scene's image ceiling. No crate, because nothing here may import from the host.

/// An RGB image as PNG bytes.
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for row in rgb.chunks(width as usize * 3).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for x in chunk {
            a += u32::from(*x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// LSB-first bit packing, as deflate wants.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u32, count: u32) {
        self.acc |= value << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// A Huffman code, which deflate stores most significant bit first.
    fn code(&mut self, code: u32, len: u32) {
        let reversed = code.reverse_bits() >> (32 - len);
        self.put(reversed, len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

fn literal(bits: &mut Bits, sym: u32) {
    match sym {
        0..=143 => bits.code(0x30 + sym, 8),
        144..=255 => bits.code(0x190 + sym - 144, 9),
        256..=279 => bits.code(sym - 256, 7),
        _ => bits.code(0xc0 + sym - 280, 8),
    }
}

const LEN_BASE: [u32; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u32; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385,
    24577,
];
const DIST_EXTRA: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn matched(bits: &mut Bits, len: u32, dist: u32) {
    let li = LEN_BASE.iter().rposition(|b| *b <= len).unwrap_or(0);
    literal(bits, 257 + li as u32);
    bits.put(len - LEN_BASE[li], LEN_EXTRA[li]);
    let di = DIST_BASE.iter().rposition(|b| *b <= dist).unwrap_or(0);
    bits.code(di as u32, 5);
    bits.put(dist - DIST_BASE[di], DIST_EXTRA[di]);
}

/// One fixed-Huffman deflate block over `data`, with a one-slot hash table finding repeats.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    const WINDOW: usize = 32_768;
    const MAX_LEN: usize = 258;
    let mut bits = Bits { out: Vec::new(), acc: 0, n: 0 };
    bits.put(1, 1);
    bits.put(1, 2);
    let mut table = vec![usize::MAX; 1 << 13];
    let hash = |i: usize| {
        let v = u32::from(data[i]) << 16 | u32::from(data[i + 1]) << 8 | u32::from(data[i + 2]);
        (v.wrapping_mul(0x9e37_79b1) >> 19) as usize
    };
    let mut i = 0;
    while i < data.len() {
        let mut best = 0;
        let mut dist = 0;
        if i + 3 <= data.len() {
            let h = hash(i);
            let cand = table[h];
            table[h] = i;
            if cand != usize::MAX && i - cand <= WINDOW {
                let limit = MAX_LEN.min(data.len() - i);
                while best < limit && data[cand + best] == data[i + best] {
                    best += 1;
                }
                dist = i - cand;
            }
        }
        if best >= 3 {
            matched(&mut bits, best as u32, dist as u32);
            for k in i + 1..(i + best).min(data.len().saturating_sub(2)) {
                table[hash(k)] = k;
            }
            i += best;
        } else {
            literal(&mut bits, u32::from(data[i]));
            i += 1;
        }
    }
    literal(&mut bits, 256);
    bits.finish()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    out.extend(deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_round_trips_through_a_real_inflater() {
        let mut samples: Vec<Vec<u8>> = vec![vec![], b"a".to_vec(), b"abcabcabcabcabcabc".to_vec(), vec![7; 5000]];
        samples.push((0..20_000u32).map(|i| (i * 31 % 251) as u8).collect());
        samples.push((0..70_000u32).map(|i| ((i / 300) % 3) as u8 * 40).collect());
        for data in samples {
            let packed = zlib(&data);
            let back = miniz_oxide::inflate::decompress_to_vec_zlib(&packed).expect("inflates");
            assert_eq!(back, data);
        }
    }

    #[test]
    fn repetitive_pictures_shrink() {
        let data = vec![9u8; 30_000];
        assert!(deflate(&data).len() < 600, "{}", deflate(&data).len());
    }

    #[test]
    fn a_png_has_its_signature_chunks_and_crcs() {
        let png = encode_rgb(2, 2, &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]));
        let idat_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&png[41..41 + idat_len]).unwrap();
        assert_eq!(raw, vec![0, 255, 0, 0, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255]);
    }
}
