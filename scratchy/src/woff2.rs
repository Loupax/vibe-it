//! QFF1b: WOFF 2.0 (W3C REC 2018) to sfnt. The brotli stream is decoded by the `brotli-decompressor` crate
//! (safe Rust, no sockets); the table directory, the glyf/loca transform (glyphs rebuilt into plain TrueType
//! glyph records) and the hmtx transform are done here.
//!
//! Hostile input: every size is checked against the bytes that are really there, the brotli output is read
//! through `take(cap)`, no allocation is sized from a header claim, and any malformation returns `None`
//! (the face then fails like any undecodable font and the page keeps its system fonts). Never panics.

use std::io::Read;

/// Largest decompressed table payload accepted (and the largest sfnt written). Real web fonts are far below.
const MAX_DECODED: usize = 32 << 20;
/// Largest `numTables` (the sfnt writer sorts and pads them).
const MAX_TABLES: usize = 256;

const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm", b"glyf", b"loca", b"prep", b"CFF ", b"VORG",
    b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern", b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar", b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc",
    b"feat", b"fmtx", b"fvar", b"gvar", b"hsty", b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

/// A forward-only byte cursor; every read is bounds-checked.
struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8]) -> Self {
        Cur { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.p.checked_add(n)?;
        let s = self.b.get(self.p..end)?;
        self.p = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        let s = self.take(2)?;
        Some(u16::from_be_bytes([s[0], s[1]]))
    }
    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.take(4)?;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// UIntBase128 (WOFF2 section 5.1).
    fn base128(&mut self) -> Option<u32> {
        let mut acc: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            if i == 0 && b == 0x80 {
                return None;
            }
            if acc & 0xFE00_0000 != 0 {
                return None;
            }
            acc = (acc << 7) | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Some(acc);
            }
        }
        None
    }
    /// 255UInt16 (WOFF2 section 5.2).
    fn u255(&mut self) -> Option<u16> {
        let c = self.u8()?;
        match c {
            253 => self.u16(),
            255 => Some(u16::from(self.u8()?) + 253),
            254 => Some(u16::from(self.u8()?) + 506),
            v => Some(u16::from(v)),
        }
    }
}

struct Entry {
    tag: [u8; 4],
    orig_len: usize,
    /// Length inside the decompressed stream (the transform length when transformed).
    stream_len: usize,
    transformed: bool,
}

/// Decode a WOFF2 file (not a collection) into sfnt bytes. `None` for anything malformed or too large.
pub(crate) fn woff2_to_sfnt(b: &[u8]) -> Option<Vec<u8>> {
    let mut c = Cur::new(b);
    if c.take(4)? != b"wOF2" {
        return None;
    }
    let flavor = c.u32()?;
    if flavor == u32::from_be_bytes(*b"ttcf") {
        return None; // collections are not supported
    }
    let _length = c.u32()?;
    let num_tables = usize::from(c.u16()?);
    c.u16()?; // reserved
    let _total_sfnt = c.u32()?; // a claim only: never used for sizing
    let comp_size = c.u32()? as usize;
    c.take(24)?; // version, metadata and private block fields
    if num_tables == 0 || num_tables > MAX_TABLES {
        return None;
    }
    let mut entries: Vec<Entry> = Vec::with_capacity(num_tables);
    let mut total: usize = 0;
    for _ in 0..num_tables {
        let flags = c.u8()?;
        let idx = usize::from(flags & 0x3F);
        let tag: [u8; 4] = if idx == 63 {
            let t = c.take(4)?;
            [t[0], t[1], t[2], t[3]]
        } else {
            **KNOWN_TAGS.get(idx)?
        };
        let version = flags >> 6;
        let orig_len = c.base128()? as usize;
        let is_glyf_loca = &tag == b"glyf" || &tag == b"loca";
        let transformed = if is_glyf_loca { version == 0 } else { version != 0 };
        if is_glyf_loca && version > 3 || (!is_glyf_loca && &tag == b"hmtx" && version > 1) {
            return None;
        }
        let stream_len = if transformed { c.base128()? as usize } else { orig_len };
        if orig_len > MAX_DECODED || stream_len > MAX_DECODED {
            return None;
        }
        total = total.checked_add(stream_len)?;
        if total > MAX_DECODED {
            return None;
        }
        entries.push(Entry { tag, orig_len, stream_len, transformed });
    }
    // Brotli stream, read through a hard cap.
    let comp = c.take(comp_size)?;
    let mut data: Vec<u8> = Vec::new();
    let mut dec = brotli_decompressor::Decompressor::new(comp, 4096).take(total as u64 + 1);
    dec.read_to_end(&mut data).ok()?;
    if data.len() != total {
        return None;
    }
    // Slice the decompressed stream per table.
    let mut slices: Vec<&[u8]> = Vec::with_capacity(entries.len());
    let mut at = 0usize;
    for e in &entries {
        slices.push(&data[at..at + e.stream_len]);
        at += e.stream_len;
    }
    let find = |tag: &[u8; 4]| entries.iter().position(|e| &e.tag == tag);

    let mut out_tables: Vec<([u8; 4], Vec<u8>)> = Vec::with_capacity(entries.len());
    let mut glyf_loca: Option<(Vec<u8>, Vec<u8>, Vec<i16>, u16)> = None; // glyf, loca, per-glyph xMin, indexFormat
    if let Some(gi) = find(b"glyf") {
        if entries[gi].transformed {
            let li = find(b"loca")?;
            if !entries[li].transformed || entries[li].stream_len != 0 {
                return None;
            }
            let r = reconstruct_glyf(slices[gi])?;
            if r.0.len() != entries[gi].orig_len && entries[gi].orig_len != 0 {
                // The directory's origLength is advisory for transformed tables; only a gross lie is rejected.
                if r.0.len() > MAX_DECODED {
                    return None;
                }
            }
            glyf_loca = Some(r);
        }
    } else if find(b"loca").is_some_and(|i| entries[i].transformed) {
        return None;
    }
    for (i, e) in entries.iter().enumerate() {
        let body: Vec<u8> = if &e.tag == b"glyf" && e.transformed {
            glyf_loca.as_ref()?.0.clone()
        } else if &e.tag == b"loca" && e.transformed {
            glyf_loca.as_ref()?.1.clone()
        } else if &e.tag == b"hmtx" && e.transformed {
            let (_, _, xmins, _) = glyf_loca.as_ref()?;
            let hhea = find(b"hhea")?;
            let maxp = find(b"maxp")?;
            let num_hm = Cur { b: slices[hhea], p: 34 }.u16()? as usize;
            let num_glyphs = Cur { b: slices[maxp], p: 4 }.u16()? as usize;
            rebuild_hmtx(slices[i], num_hm, num_glyphs, xmins)?
        } else {
            if e.stream_len != e.orig_len {
                return None;
            }
            let mut v = slices[i].to_vec();
            if &e.tag == b"head" {
                if let Some((_, _, _, fmt)) = &glyf_loca {
                    if v.len() < 52 {
                        return None;
                    }
                    v[50..52].copy_from_slice(&fmt.to_be_bytes());
                }
            }
            v
        };
        out_tables.push((e.tag, body));
    }
    write_sfnt(flavor, out_tables)
}

fn write_sfnt(flavor: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Option<Vec<u8>> {
    tables.sort_by_key(|t| t.0);
    let n = tables.len();
    let dir_end = 12 + 16 * n;
    let body: usize = tables.iter().map(|t| (t.1.len() + 3) & !3).sum();
    if dir_end + body > MAX_DECODED + (1 << 20) {
        return None;
    }
    let mut out = Vec::with_capacity(dir_end + body);
    let mut entry_sel = 0u16;
    while (2usize << entry_sel) <= n {
        entry_sel += 1;
    }
    let search_range = 16u16 << entry_sel;
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_sel.to_be_bytes());
    out.extend_from_slice(&((n as u16) * 16 - search_range).to_be_bytes());
    let mut off = dir_end;
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(off as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        off += (data.len() + 3) & !3;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Some(out)
}

fn checksum(d: &[u8]) -> u32 {
    let mut sum = 0u32;
    for ch in d.chunks(4) {
        let mut w = [0u8; 4];
        w[..ch.len()].copy_from_slice(ch);
        sum = sum.wrapping_add(u32::from_be_bytes(w));
    }
    sum
}

fn rebuild_hmtx(t: &[u8], num_hm: usize, num_glyphs: usize, xmins: &[i16]) -> Option<Vec<u8>> {
    let mut c = Cur::new(t);
    let flags = c.u8()?;
    if num_hm == 0 || num_hm > num_glyphs || num_glyphs != xmins.len() {
        return None;
    }
    let mut adv = Vec::with_capacity(num_hm);
    for _ in 0..num_hm {
        adv.push(c.u16()?);
    }
    let mut lsb: Vec<i16> = Vec::with_capacity(num_glyphs);
    if flags & 1 == 0 {
        for _ in 0..num_hm {
            lsb.push(c.i16()?);
        }
    } else {
        lsb.extend_from_slice(&xmins[..num_hm]);
    }
    if flags & 2 == 0 {
        for _ in num_hm..num_glyphs {
            lsb.push(c.i16()?);
        }
    } else {
        lsb.extend_from_slice(&xmins[num_hm..]);
    }
    let mut out = Vec::with_capacity(num_hm * 4 + (num_glyphs - num_hm) * 2);
    for i in 0..num_glyphs {
        if i < num_hm {
            out.extend_from_slice(&adv[i].to_be_bytes());
        }
        out.extend_from_slice(&lsb[i].to_be_bytes());
    }
    Some(out)
}

fn with_sign(flag: u8, v: i32) -> i32 {
    if flag & 1 != 0 { v } else { -v }
}

/// Rebuild `glyf` and `loca` from the transformed glyf table. Returns (glyf, loca, per-glyph xMin, indexFormat).
fn reconstruct_glyf(t: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<i16>, u16)> {
    let mut h = Cur::new(t);
    if h.u16()? != 0 {
        return None; // transform version
    }
    let option_flags = h.u16()?;
    let num_glyphs = usize::from(h.u16()?);
    let index_format = h.u16()?;
    if index_format > 1 {
        return None;
    }
    let mut sizes = [0usize; 7];
    for s in sizes.iter_mut() {
        *s = h.u32()? as usize;
    }
    let mut take = |n: usize| -> Option<Cur<'_>> { h.take(n).map(Cur::new) };
    let mut n_contour = take(sizes[0])?;
    let mut n_points = take(sizes[1])?;
    let mut flag_s = take(sizes[2])?;
    let mut glyph_s = take(sizes[3])?;
    let mut comp_s = take(sizes[4])?;
    let bbox_all = take(sizes[5])?;
    let mut instr_s = take(sizes[6])?;
    let overlap: Option<&[u8]> = if option_flags & 1 != 0 { Some(h.take(num_glyphs.div_ceil(8))?) } else { None };

    let bitmap_len = ((num_glyphs + 31) >> 5) << 2;
    let mut bbox_c = bbox_all;
    let bitmap = bbox_c.take(bitmap_len)?;

    let mut glyf: Vec<u8> = Vec::new();
    let mut offsets: Vec<u32> = Vec::with_capacity(num_glyphs + 1);
    let mut xmins: Vec<i16> = Vec::with_capacity(num_glyphs);
    for g in 0..num_glyphs {
        offsets.push(u32::try_from(glyf.len()).ok()?);
        let nc = n_contour.i16()?;
        let has_bbox = bitmap[g >> 3] & (0x80 >> (g & 7)) != 0;
        let start = glyf.len();
        if nc == 0 {
            if has_bbox {
                return None;
            }
            xmins.push(0);
            continue;
        }
        if nc > 0 {
            let nc = nc as usize;
            let mut end_pts: Vec<u16> = Vec::with_capacity(nc);
            let mut npts = 0usize;
            for _ in 0..nc {
                npts = npts.checked_add(usize::from(n_points.u255()?))?;
                if npts == 0 || npts > 0xFFFF + 1 {
                    return None;
                }
                end_pts.push((npts - 1) as u16);
            }
            let flags = flag_s.take(npts)?;
            let mut xs: Vec<i32> = Vec::with_capacity(npts);
            let mut ys: Vec<i32> = Vec::with_capacity(npts);
            let (mut x, mut y) = (0i32, 0i32);
            for &f in flags {
                let idx = f & 0x7F;
                let (dx, dy) = triplet(idx, &mut glyph_s)?;
                x = x.checked_add(dx)?;
                y = y.checked_add(dy)?;
                xs.push(x);
                ys.push(y);
            }
            let instr_len = usize::from(glyph_s.u255()?);
            let instr = instr_s.take(instr_len)?;
            let (xmin, ymin, xmax, ymax) = if has_bbox {
                (bbox_c.i16()?, bbox_c.i16()?, bbox_c.i16()?, bbox_c.i16()?)
            } else {
                (
                    i16::try_from(*xs.iter().min()?).ok()?,
                    i16::try_from(*ys.iter().min()?).ok()?,
                    i16::try_from(*xs.iter().max()?).ok()?,
                    i16::try_from(*ys.iter().max()?).ok()?,
                )
            };
            xmins.push(xmin);
            glyf.extend_from_slice(&(nc as i16).to_be_bytes());
            for v in [xmin, ymin, xmax, ymax] {
                glyf.extend_from_slice(&v.to_be_bytes());
            }
            for e in &end_pts {
                glyf.extend_from_slice(&e.to_be_bytes());
            }
            glyf.extend_from_slice(&(instr_len as u16).to_be_bytes());
            glyf.extend_from_slice(instr);
            let ov = overlap.is_some_and(|o| o[g >> 3] & (1 << (g & 7)) != 0);
            for (i, &f) in flags.iter().enumerate() {
                let mut fl = if f & 0x80 == 0 { 1u8 } else { 0 };
                if i == 0 && ov {
                    fl |= 0x40;
                }
                glyf.push(fl);
            }
            for coords in [&xs, &ys] {
                let mut prev = 0i32;
                for &v in coords.iter() {
                    let d = i16::try_from(v - prev).ok()?;
                    glyf.extend_from_slice(&d.to_be_bytes());
                    prev = v;
                }
            }
        } else if nc == -1 {
            if !has_bbox {
                return None;
            }
            let (xmin, ymin, xmax, ymax) = (bbox_c.i16()?, bbox_c.i16()?, bbox_c.i16()?, bbox_c.i16()?);
            xmins.push(xmin);
            glyf.extend_from_slice(&(-1i16).to_be_bytes());
            for v in [xmin, ymin, xmax, ymax] {
                glyf.extend_from_slice(&v.to_be_bytes());
            }
            let comp_start = comp_s.p;
            let mut have_instr = false;
            loop {
                let fl = comp_s.u16()?;
                comp_s.u16()?; // glyph index
                let mut skip = if fl & 0x0001 != 0 { 4 } else { 2 };
                if fl & 0x0008 != 0 {
                    skip += 2;
                } else if fl & 0x0040 != 0 {
                    skip += 4;
                } else if fl & 0x0080 != 0 {
                    skip += 8;
                }
                comp_s.take(skip)?;
                if fl & 0x0100 != 0 {
                    have_instr = true;
                }
                if fl & 0x0020 == 0 {
                    break;
                }
            }
            glyf.extend_from_slice(&comp_s.b[comp_start..comp_s.p]);
            if have_instr {
                let n = usize::from(glyph_s.u255()?);
                glyf.extend_from_slice(&(n as u16).to_be_bytes());
                glyf.extend_from_slice(instr_s.take(n)?);
            }
        } else {
            return None;
        }
        if glyf.len() > MAX_DECODED || glyf.len() < start {
            return None;
        }
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
    }
    offsets.push(u32::try_from(glyf.len()).ok()?);
    // Our glyph records are uncompressed (wider than the woff2 ones): a short loca can overflow, use the long one then.
    let index_format = if index_format == 0 && offsets.last().is_some_and(|&o| o / 2 > 0xFFFF) { 1 } else { index_format };
    let mut loca = Vec::with_capacity((num_glyphs + 1) * 4);
    for o in &offsets {
        if index_format == 0 {
            loca.extend_from_slice(&((o / 2) as u16).to_be_bytes());
        } else {
            loca.extend_from_slice(&o.to_be_bytes());
        }
    }
    Some((glyf, loca, xmins, index_format))
}

fn triplet(f: u8, s: &mut Cur<'_>) -> Option<(i32, i32)> {
    let fi = i32::from(f);
    Some(if f < 10 {
        let b0 = i32::from(s.u8()?);
        (0, with_sign(f, ((fi & 14) << 7) + b0))
    } else if f < 20 {
        let b0 = i32::from(s.u8()?);
        (with_sign(f, (((fi - 10) & 14) << 7) + b0), 0)
    } else if f < 84 {
        let b0 = fi - 20;
        let b1 = i32::from(s.u8()?);
        (with_sign(f, 1 + (b0 & 0x30) + (b1 >> 4)), with_sign(f >> 1, 1 + ((b0 & 0x0C) << 2) + (b1 & 0x0F)))
    } else if f < 120 {
        let b0 = fi - 84;
        let (b1, b2) = (i32::from(s.u8()?), i32::from(s.u8()?));
        (with_sign(f, 1 + ((b0 / 12) << 8) + b1), with_sign(f >> 1, 1 + (((b0 % 12) >> 2) << 8) + b2))
    } else if f < 124 {
        let (b1, b2, b3) = (i32::from(s.u8()?), i32::from(s.u8()?), i32::from(s.u8()?));
        (with_sign(f, (b1 << 4) + (b2 >> 4)), with_sign(f >> 1, ((b2 & 0x0F) << 8) + b3))
    } else {
        let (b1, b2, b3, b4) = (i32::from(s.u8()?), i32::from(s.u8()?), i32::from(s.u8()?), i32::from(s.u8()?));
        (with_sign(f, (b1 << 8) + b2), with_sign(f >> 1, (b3 << 8) + b4))
    })
}
