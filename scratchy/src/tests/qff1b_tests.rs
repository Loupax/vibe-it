//! QFF1b: woff2 support (src/woff2.rs): a generated woff2 of the test box font (transformed glyf/loca and
//! hmtx, brotli stored in uncompressed meta-blocks), truncated and oversized-claim files fail cleanly.
use crate::webfonts as wf;

use super::qff1a_tests::box_font_tables;

/// A brotli stream made of uncompressed meta-blocks (valid, no compressor needed).
fn brotli_stored(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut first = true;
    for chunk in data.chunks(65536) {
        let mut bits: Vec<bool> = Vec::new();
        if first {
            bits.push(false); // WBITS = 16
        }
        first = false;
        bits.push(false); // ISLAST = 0
        bits.extend([false, false]); // MNIBBLES = 4
        let m = chunk.len() - 1;
        for i in 0..16 {
            bits.push(m >> i & 1 != 0);
        }
        bits.push(true); // ISUNCOMPRESSED
        for ch in bits.chunks(8) {
            let mut b = 0u8;
            for (i, &v) in ch.iter().enumerate() {
                b |= (v as u8) << i;
            }
            out.push(b);
        }
        out.extend_from_slice(chunk);
    }
    if out.is_empty() {
        out.push(0x01); // WBITS = 16 then ISLAST ISLASTEMPTY
    } else {
        out.push(0x03); // ISLAST = 1, ISLASTEMPTY = 1
    }
    out
}

fn base128(mut v: u32) -> Vec<u8> {
    let mut parts = vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 {
        parts.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    parts.reverse();
    parts
}

/// The box font's transformed glyf table: glyph 0 empty, glyph 1 one contour of four points.
fn transformed_glyf() -> Vec<u8> {
    let n_contour = [0u8, 0, 0, 1];
    let n_points = [4u8];
    let flags = [124u8, 127, 127, 126];
    let glyph = [0u8, 0, 0, 200, 0x03, 0xE8, 0, 0, 0, 0, 0x03, 0xE8, 0x03, 0xE8, 0, 0, 0 /* instruction length */];
    let bbox_bitmap = [0u8; 4];
    let mut t = Vec::new();
    for v in [0u16, 0, 2, 0] {
        t.extend_from_slice(&v.to_be_bytes()); // version, option flags, numGlyphs, indexFormat (short)
    }
    for len in [n_contour.len(), n_points.len(), flags.len(), glyph.len(), 0, bbox_bitmap.len(), 0] {
        t.extend_from_slice(&(len as u32).to_be_bytes());
    }
    for s in [&n_contour[..], &n_points, &flags, &glyph, &bbox_bitmap] {
        t.extend_from_slice(s);
    }
    t
}

/// The woff2 of the box font: glyf/loca/hmtx transformed when `transform`. Returns (file, header+directory length).
fn woff2_parts(family: &str, transform: bool, stream_override: Option<Vec<u8>>) -> (Vec<u8>, usize) {
    let tables = box_font_tables(family);
    let known = ["cmap", "head", "hhea", "hmtx", "maxp", "name", "OS/2", "post", "cvt ", "fpgm", "glyf", "loca"];
    let mut dir = Vec::new();
    let mut stream = Vec::new();
    for (tag, data) in &tables {
        let tag_s = std::str::from_utf8(tag).unwrap();
        let idx = known.iter().position(|k| *k == tag_s).unwrap() as u8;
        let (body, version, tlen): (Vec<u8>, u8, Option<usize>) = match (tag_s, transform) {
            ("glyf", true) => {
                let t = transformed_glyf();
                let l = t.len();
                (t, 0, Some(l))
            }
            ("loca", true) => (Vec::new(), 0, Some(0)),
            ("hmtx", true) => (vec![3, 0x03, 0xE8, 0x03, 0xE8], 1, Some(5)),
            ("glyf" | "loca", false) => (data.clone(), 3, None),
            _ => (data.clone(), 0, None),
        };
        dir.push(idx | version << 6);
        dir.extend(base128(data.len() as u32));
        if let Some(l) = tlen {
            dir.extend(base128(l as u32));
        }
        stream.extend_from_slice(&body);
    }
    let comp = brotli_stored(stream_override.as_deref().unwrap_or(&stream));
    let mut out = Vec::new();
    out.extend_from_slice(b"wOF2");
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // length (patched below)
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // totalSfntSize (a claim)
    out.extend_from_slice(&(comp.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0u8; 24]);
    out.extend_from_slice(&dir);
    let head = out.len();
    out.extend_from_slice(&comp);
    let l = out.len() as u32;
    out[8..12].copy_from_slice(&l.to_be_bytes());
    (out, head)
}

fn woff2(family: &str, transform: bool) -> Vec<u8> {
    woff2_parts(family, transform, None).0
}

/// (tag, bytes) of an sfnt's tables.
fn read_tables(b: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let n = u16::from_be_bytes([b[4], b[5]]) as usize;
    (0..n)
        .map(|i| {
            let e = 12 + 16 * i;
            let tag = [b[e], b[e + 1], b[e + 2], b[e + 3]];
            let off = u32::from_be_bytes(b[e + 8..e + 12].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(b[e + 12..e + 16].try_into().unwrap()) as usize;
            (tag, b[off..off + len].to_vec())
        })
        .collect()
}

#[test]
fn qff1b_woff2_transformed_decodes_to_the_box_font() {
    let orig = box_font_tables("Woff2 Box");
    let w = woff2("Woff2 Box", true);
    let sfnt = crate::woff2::woff2_to_sfnt(&w).expect("decodes");
    let got = read_tables(&sfnt);
    for (tag, data) in &orig {
        let g = &got.iter().find(|t| t.0 == *tag).unwrap().1;
        if tag == b"glyf" {
            assert_eq!(&g[..data.len()], &data[..], "glyph records rebuilt exactly (padding follows)");
        } else if tag == b"loca" {
            assert_eq!(g.len(), data.len());
            assert_eq!(&g[..4], &data[..4]);
        } else {
            assert_eq!(g, data, "table {}", String::from_utf8_lossy(tag));
        }
    }
    let face = wf::decode_face(w).expect("a face");
    assert_eq!(face.families[0].0, "Woff2 Box");
}

#[test]
fn qff1b_woff2_null_transform_decodes() {
    let face = wf::decode_face(woff2("Plain Box", false)).expect("a face");
    assert_eq!(face.families[0].0, "Plain Box");
}

#[test]
fn qff1b_truncated_woff2_fails_cleanly() {
    let w = woff2("Cut Box", true);
    for n in 0..w.len() {
        assert!(wf::decode_face(w[..n].to_vec()).is_none(), "prefix of {n} bytes");
    }
}

#[test]
fn qff1b_oversized_claims_are_refused_without_allocating() {
    // A 2 GiB origLength on the first (cmap, null transform) directory entry.
    let (mut w, _) = woff2_parts("Big Box", true, None);
    let old_len = w[49..].iter().position(|v| v & 0x80 == 0).unwrap() + 1;
    w.splice(49..49 + old_len, base128(0x7FFF_FFFF));
    assert!(crate::woff2::woff2_to_sfnt(&w).is_none());
    // More brotli output than the directory declares (a decompression bomb in miniature).
    let (w, _) = woff2_parts("Bomb Box", false, Some(vec![0u8; 1 << 20]));
    assert!(crate::woff2::woff2_to_sfnt(&w).is_none(), "output longer than declared");
    // Too many tables.
    let mut w = woff2("Few Box", true);
    w[12..14].copy_from_slice(&1000u16.to_be_bytes());
    assert!(crate::woff2::woff2_to_sfnt(&w).is_none());
    // A glyph count with no data behind it (transformed glyf claiming 65535 glyphs).
    let mut g = transformed_glyf();
    g[4..6].copy_from_slice(&65535u16.to_be_bytes());
    assert!(crate::woff2::woff2_to_sfnt(&woff2_with_glyf("Many Box", g)).is_none());
}

/// The transformed box woff2 with the glyf transform table replaced by one of the same length.
fn woff2_with_glyf(family: &str, glyf: Vec<u8>) -> Vec<u8> {
    let mut stream = Vec::new();
    for (tag, data) in &box_font_tables(family) {
        match tag {
            b"glyf" => stream.extend_from_slice(&glyf),
            b"loca" => {}
            b"hmtx" => stream.extend_from_slice(&[3, 0x03, 0xE8, 0x03, 0xE8]),
            _ => stream.extend_from_slice(data),
        }
    }
    woff2_parts(family, true, Some(stream)).0
}

#[test]
fn qff1b_corrupted_woff2_never_panics() {
    let w = woff2("Fuzz Box", true);
    let mut seed = 0x1234_5678u32;
    for _ in 0..4000 {
        let mut c = w.clone();
        for _ in 0..1 + seed % 3 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let i = (seed >> 8) as usize % c.len();
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            c[i] = (seed >> 16) as u8;
        }
        let _ = wf::decode_face(c);
    }
}

#[test]
fn qff1b_real_woff2_files_when_available() {
    let dir = std::path::Path::new("/home/loupax/src/wpt/fonts/noto");
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut n = 0;
    for e in rd.flatten() {
        if e.path().extension().is_some_and(|x| x == "woff2") {
            let b = std::fs::read(e.path()).unwrap();
            let face = wf::decode_face(b).unwrap_or_else(|| panic!("{} decodes", e.path().display()));
            assert!(!face.families.is_empty());
            n += 1;
        }
    }
    eprintln!("decoded {n} real woff2 files");
}
