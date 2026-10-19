//! Reader/writer for Brother ScanNCut `.fcm` (`#FCM0100`) files.
//!
//! Layout recovered from libFCM.dylib (`ReadFcmData` and the `Read*Section` helpers, decompiled with
//! Ghidra through REA) and checked against sample files plus Brother's own library used as an oracle.
//! All integers are little-endian. Coordinates are 1/100 mm.
//!
//! Fields whose meaning is not established are kept as `unk*` so files round-trip byte for byte.

pub mod build;

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Eof { at: usize, need: usize },
    BadMagic,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Eof { at, need } => write!(f, "unexpected end of file at {at} (need {need} bytes)"),
            Error::BadMagic => write!(f, "not an #FCM file"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

struct Rd<'a> {
    b: &'a [u8],
    pos: usize,
}
impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.b.len() {
            return Err(Error::Eof { at: self.pos, need: n });
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        let at = self.pos;
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N).map_err(|_| Error::Eof { at, need: N })?);
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.arr::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    /// A count read from the file, as a capacity that cannot exceed what the rest of the file could hold.
    fn cap(&self, n: usize, min_item: usize) -> usize {
        n.min(self.b.len().saturating_sub(self.pos) / min_item.max(1))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileHeader {
    pub format: [u8; 4],  // "#FCM"
    pub version: [u8; 4], // "0100"
    pub content_id: u32,
    pub short_name: [u8; 8],
    /// UTF-16 code units (stored with a one-byte character count).
    pub long_name: Vec<u16>,
    pub author: Vec<u16>,
    pub copyright: Vec<u16>,
    pub thumb_block_w: u8,
    pub thumb_block_h: u8,
    /// Embedded 1-bit BMP preview.
    pub thumbnail: Vec<u8>,
    pub machine_model_id: [u8; 4], // e.g. "1APP"
    pub save_machine_ver: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CutHeader {
    pub mat_id: u32,
    pub cut_area_width: u32,
    pub cut_area_height: u32,
    pub seam_allowance_width: u32,
    /// Present only when the block is longer than 16 bytes: align_flag, mark count, 4 x (x, y).
    pub align: Option<[i32; 10]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Line { x: i32, y: i32 },
    Bezier { c1x: i32, c1y: i32, c2x: i32, c2y: i32, x: i32, y: i32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// 1 = bezier segments, anything else = line segments.
    pub kind: u32,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathKind {
    Line { unk0: u32, unk1_f32_bits: u32, start_x: i32, start_y: i32, outlines: Vec<Outline> },
    Rhinestone { attribute: u32, radius: u32, points: Vec<(i32, i32)> },
}

/// Bits of [`Path::flags`], recovered by setting each bit in a file and decoding it with Brother's
/// own library (`re/oracle`). Bits 8-11 are unused.
pub mod flags {
    pub const OPEN: u32 = 0x1;
    pub const CUT: u32 = 0x2;
    pub const DRAW: u32 = 0x4;
    pub const TARGET_SEAM_ALLOWANCE: u32 = 0x8;
    /// Rhinestone segment (points instead of outlines).
    pub const RHINESTONE: u32 = 0x10;
    pub const PAINT_FACE_WHEN_DRAWING: u32 = 0x20;
    pub const SCANNER_CORRECTION: u32 = 0x40;
    pub const ADDITIONAL_LINE_AT_DRAW: u32 = 0x80;
    pub const ONLY_FOR_DRAW: u32 = 0x1000;
    pub const EMBOSS: u32 = 0x2000;
    pub const FOIL: u32 = 0x4000;
    /// Stored as a points segment, like rhinestones.
    pub const PERFORATING: u32 = 0x8000;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// Flag word (bit 0x8010 selects the rhinestone variant).
    pub flags: u32,
    pub kind: PathKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub unk0: u32,
    pub unk1: u32,
    pub width: i32,
    pub height: i32,
    pub unk4: u32,
    /// Affine matrix a11 a21 a12 a22 a13 a23.
    pub affine: [f32; 6],
    pub unk11: u32,
    pub unk12: u32,
    pub unk13: u32,
    pub flags: [u8; 4],
    pub paths: Vec<Path>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fcm {
    pub header: FileHeader,
    pub cut: CutHeader,
    pub piece_labels: Vec<u16>,
    pub pieces: Vec<Piece>,
    /// Bytes after the last piece, kept verbatim.
    pub tail: Vec<u8>,
}

fn rd_utf16(r: &mut Rd) -> Result<Vec<u16>> {
    let n = r.u8()? as usize;
    let raw = r.take(n * 2)?;
    Ok(raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
}

pub fn parse(b: &[u8]) -> Result<Fcm> {
    let mut r = Rd { b, pos: 0 };
    let format: [u8; 4] = r.arr()?;
    if &format != b"#FCM" {
        return Err(Error::BadMagic);
    }
    let version: [u8; 4] = r.arr()?;
    let content_id = r.u32()?;
    let hdr_len = r.u32()? as usize;
    let short_name: [u8; 8] = r.arr()?;
    let long_name = rd_utf16(&mut r)?;
    let author = rd_utf16(&mut r)?;
    let copyright = rd_utf16(&mut r)?;
    let thumb_block_w = r.u8()?;
    let thumb_block_h = r.u8()?;
    let tlen = r.u32()? as usize;
    let thumbnail = r.take(tlen)?.to_vec();
    let machine_model_id: [u8; 4] = r.arr()?;
    let save_machine_ver = r.u32()?;
    let header = FileHeader {
        format,
        version,
        content_id,
        short_name,
        long_name,
        author,
        copyright,
        thumb_block_w,
        thumb_block_h,
        thumbnail,
        machine_model_id,
        save_machine_ver,
    };

    let cut_pos = hdr_len + 0x10;
    r.pos = cut_pos;
    let cl = r.u32()? as usize;
    let mat_id = r.u32()?;
    let cut_area_width = r.u32()?;
    let cut_area_height = r.u32()?;
    let seam_allowance_width = r.u32()?;
    let align = if r.pos < cut_pos + cl + 4 {
        let mut a = [0i32; 10];
        for v in a.iter_mut() {
            *v = r.i32()?;
        }
        Some(a)
    } else {
        None
    };
    let cut = CutHeader { mat_id, cut_area_width, cut_area_height, seam_allowance_width, align };

    r.pos = hdr_len + cl + 0x14;
    let n = r.u32()? as usize;
    let mut offs = Vec::with_capacity(r.cap(n, 4));
    for _ in 0..n {
        offs.push(r.u32()? as usize);
    }
    let _pieces_total = r.u32()?;
    let k = r.u32()? as usize;
    let mut piece_labels = Vec::with_capacity(r.cap(k, 2));
    for _ in 0..k {
        piece_labels.push(r.u16()?);
    }
    let base = r.pos;
    let mut pieces = Vec::new();
    let mut end = base;
    for &o in &offs {
        let (p, e) = parse_piece(b, base + o)?;
        end = end.max(e);
        pieces.push(p);
    }
    let tail = b[end.min(b.len())..].to_vec();
    Ok(Fcm { header, cut, piece_labels, pieces, tail })
}

fn parse_piece(b: &[u8], at: usize) -> Result<(Piece, usize)> {
    let mut r = Rd { b, pos: at };
    let unk0 = r.u32()?;
    let unk1 = r.u32()?;
    let width = r.i32()?;
    let height = r.i32()?;
    let unk4 = r.u32()?;
    let mut affine = [0f32; 6];
    for v in affine.iter_mut() {
        *v = f32::from_bits(r.u32()?);
    }
    let unk11 = r.u32()?;
    let unk12 = r.u32()?;
    let unk13 = r.u32()?;
    let extra = r.u32()? as usize; // size of the flag block that follows (4)
    let flags: [u8; 4] = r.arr()?;
    r.pos = at + 0x3c + extra;
    let npaths = r.u32()? as usize;
    let mut paths = Vec::with_capacity(r.cap(npaths, 12));
    for _ in 0..npaths {
        paths.push(parse_path(&mut r)?);
    }
    Ok((Piece { unk0, unk1, width, height, unk4, affine, unk11, unk12, unk13, flags, paths }, r.pos))
}

fn parse_path(r: &mut Rd) -> Result<Path> {
    let start = r.pos;
    let _len = r.u32()?;
    let extra = r.u32()? as usize;
    let flags = r.u32()?;
    r.pos = start + 8 + extra;
    let kind = if flags & 0x8010 != 0 {
        let attribute = r.u32()?;
        let count = r.u32()? as usize;
        let radius = r.u32()?;
        let mut points = Vec::with_capacity(r.cap(count, 8));
        for _ in 0..count {
            points.push((r.i32()?, r.i32()?));
        }
        PathKind::Rhinestone { attribute, radius, points }
    } else {
        let n_outlines = r.u32()? as usize;
        let unk0 = r.u32()?;
        let unk1_f32_bits = r.u32()?;
        let start_x = r.i32()?;
        let start_y = r.i32()?;
        let mut outlines = Vec::with_capacity(r.cap(n_outlines, 8));
        for _ in 0..n_outlines {
            let kind = r.u32()?;
            let count = r.u32()? as usize;
            let mut segments = Vec::with_capacity(r.cap(count, 8));
            for _ in 0..count {
                if kind == 1 {
                    segments.push(Segment::Bezier { c1x: r.i32()?, c1y: r.i32()?, c2x: r.i32()?, c2y: r.i32()?, x: r.i32()?, y: r.i32()? });
                } else {
                    segments.push(Segment::Line { x: r.i32()?, y: r.i32()? });
                }
            }
            outlines.push(Outline { kind, segments });
        }
        PathKind::Line { unk0, unk1_f32_bits, start_x, start_y, outlines }
    };
    Ok(Path { flags, kind })
}

// ---------------------------------------------------------------- writer

struct Wr(Vec<u8>);
impl Wr {
    fn u8(&mut self, v: u8) {
        self.0.push(v)
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes())
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes())
    }
    fn i32(&mut self, v: i32) {
        self.u32(v as u32)
    }
    fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v)
    }
}

fn wr_utf16(w: &mut Wr, s: &[u16]) {
    w.u8(s.len() as u8);
    for &c in s {
        w.u16(c);
    }
}

fn write_path(p: &Path) -> Vec<u8> {
    let mut body = Wr(Vec::new());
    body.u32(4); // extra
    body.u32(p.flags);
    match &p.kind {
        PathKind::Rhinestone { attribute, radius, points } => {
            body.u32(*attribute);
            body.u32(points.len() as u32);
            body.u32(*radius);
            for &(x, y) in points {
                body.i32(x);
                body.i32(y);
            }
        }
        PathKind::Line { unk0, unk1_f32_bits, start_x, start_y, outlines } => {
            body.u32(outlines.len() as u32);
            body.u32(*unk0);
            body.u32(*unk1_f32_bits);
            body.i32(*start_x);
            body.i32(*start_y);
            for o in outlines {
                body.u32(o.kind);
                body.u32(o.segments.len() as u32);
                for s in &o.segments {
                    match *s {
                        Segment::Line { x, y } => {
                            body.i32(x);
                            body.i32(y);
                        }
                        Segment::Bezier { c1x, c1y, c2x, c2y, x, y } => {
                            for v in [c1x, c1y, c2x, c2y, x, y] {
                                body.i32(v);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut out = Wr(Vec::new());
    out.u32(body.0.len() as u32);
    out.bytes(&body.0);
    out.0
}

fn write_piece(p: &Piece) -> Vec<u8> {
    let mut w = Wr(Vec::new());
    w.u32(p.unk0);
    w.u32(p.unk1);
    w.i32(p.width);
    w.i32(p.height);
    w.u32(p.unk4);
    for a in p.affine {
        w.u32(a.to_bits());
    }
    w.u32(p.unk11);
    w.u32(p.unk12);
    w.u32(p.unk13);
    w.u32(4);
    w.bytes(&p.flags);
    w.u32(p.paths.len() as u32);
    for path in &p.paths {
        w.bytes(&write_path(path));
    }
    w.0
}

pub fn write(f: &Fcm) -> Vec<u8> {
    let h = &f.header;
    // header body (after the 16 fixed bytes)
    let mut hb = Wr(Vec::new());
    hb.bytes(&h.short_name);
    wr_utf16(&mut hb, &h.long_name);
    wr_utf16(&mut hb, &h.author);
    wr_utf16(&mut hb, &h.copyright);
    hb.u8(h.thumb_block_w);
    hb.u8(h.thumb_block_h);
    hb.u32(h.thumbnail.len() as u32);
    hb.bytes(&h.thumbnail);
    hb.bytes(&h.machine_model_id);
    hb.u32(h.save_machine_ver);

    let mut w = Wr(Vec::new());
    w.bytes(&h.format);
    w.bytes(&h.version);
    w.u32(h.content_id);
    // Brother's header length covers everything up to the cut block, i.e. hb plus the 8 bytes already placed.
    w.u32(hb.0.len() as u32);
    w.bytes(&hb.0);

    // cut block
    let c = &f.cut;
    let cl: u32 = if c.align.is_some() { 16 + 40 } else { 16 };
    w.u32(cl);
    w.u32(c.mat_id);
    w.u32(c.cut_area_width);
    w.u32(c.cut_area_height);
    w.u32(c.seam_allowance_width);
    if let Some(a) = c.align {
        for v in a {
            w.i32(v);
        }
    }

    // piece table
    let blobs: Vec<Vec<u8>> = f.pieces.iter().map(write_piece).collect();
    w.u32(blobs.len() as u32);
    let mut off = 0u32;
    for b in &blobs {
        w.u32(off);
        off += b.len() as u32;
    }
    w.u32(off);
    w.u32(f.piece_labels.len() as u32);
    for &l in &f.piece_labels {
        w.u16(l);
    }
    for b in &blobs {
        w.bytes(b);
    }
    w.bytes(&f.tail);
    w.0
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    fn sample() -> Fcm {
        let ring = vec![Contour::circle(30.0), Contour::circle(12.0)];
        let mut open = Contour::polygon(&[(0.0, 0.0), (50.0, 20.0)]);
        open.closed = false;
        open.tool = Tool::Draw;
        Design {
            pieces: vec![
                PieceSpec { contours: vec![Contour::rect(60.0, 40.0)], position_mm: (60.0, 60.0) },
                PieceSpec { contours: ring, position_mm: (150.0, 80.0) },
                PieceSpec { contours: vec![open], position_mm: (100.0, 190.0) },
            ],
            ..Default::default()
        }
        .to_fcm()
    }

    #[test]
    fn builder_output_parses_back_identically_and_rewrites_the_same_bytes() {
        let f = sample();
        let bytes = write(&f);
        let back = parse(&bytes).unwrap();
        assert_eq!(back, f);
        assert_eq!(write(&back), bytes);
    }

    #[test]
    fn geometry_and_flags_survive() {
        let f = parse(&write(&sample())).unwrap();
        assert_eq!(f.pieces.len(), 3);
        assert_eq!(f.piece_labels.len(), 3);
        assert_eq!((f.pieces[0].width, f.pieces[0].height), (5999, 3999));
        assert_eq!(f.pieces[1].paths.len(), 2);
        assert_eq!(f.pieces[0].paths[0].flags, flags::CUT);
        assert_eq!(f.pieces[2].paths[0].flags, flags::DRAW | flags::OPEN);
        assert_eq!((f.cut.cut_area_width, f.cut.cut_area_height), (29667, 29880));
        assert_eq!(f.header.thumbnail.len(), 1118);
    }

    #[test]
    fn rejects_other_files() {
        assert!(matches!(parse(b"hello world, not a cutting file"), Err(Error::BadMagic)));
        assert!(parse(b"").is_err());
    }

    #[test]
    fn truncated_and_corrupt_files_error_instead_of_panicking() {
        let bytes = write(&sample());
        for cut in (0..bytes.len()).step_by(7) {
            let _ = parse(&bytes[..cut]);
        }
        // flip every byte in turn: never a panic or an enormous allocation
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            if let Some(x) = b.get_mut(i) {
                *x = !*x;
            }
            let _ = parse(&b);
        }
        // huge counts in the piece table
        let mut b = bytes.clone();
        for v in b.iter_mut().skip(0x4a0).take(8) {
            *v = 0xff;
        }
        let _ = parse(&b);
    }
}
