//! High-level builder: turn vector contours (millimetres) into a valid [`Fcm`], including the
//! 88x88 1-bit BMP preview the machine shows.
//!
//! Defaults for header fields come from a file saved by Brother's app for a CM900-class machine
//! (cut area 296.67 x 298.80 mm, 12x12 mat, model id "1APP", machine version 207).

use crate::*;

/// A contour in millimetres. Closed contours should end back at `start`.
#[derive(Debug, Clone)]
pub struct Contour {
    pub start: (f64, f64),
    pub segs: Vec<Seg>,
    /// Open contours are written with the machine's "open path" flag.
    pub closed: bool,
    pub tool: Tool,
}

/// What the machine does along a contour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Cut,
    Draw,
    Emboss,
    Foil,
}

impl Tool {
    pub fn flags(self) -> u32 {
        match self {
            Tool::Cut => flags::CUT,
            Tool::Draw => flags::DRAW,
            Tool::Emboss => flags::EMBOSS,
            Tool::Foil => flags::FOIL,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Seg {
    Line((f64, f64)),
    Cubic { c1: (f64, f64), c2: (f64, f64), to: (f64, f64) },
}

impl Contour {
    pub fn polygon(pts: &[(f64, f64)]) -> Contour {
        let Some((&first, rest)) = pts.split_first() else {
            return Contour { start: (0.0, 0.0), segs: vec![], closed: true, tool: Tool::Cut };
        };
        let mut segs: Vec<Seg> = rest.iter().map(|&p| Seg::Line(p)).collect();
        segs.push(Seg::Line(first));
        Contour { start: first, segs, closed: true, tool: Tool::Cut }
    }
    pub fn rect(w: f64, h: f64) -> Contour {
        let (x, y) = (w / 2.0, h / 2.0);
        Contour::polygon(&[(x, y), (-x, y), (-x, -y), (x, -y)])
    }
    /// Circle approximated with four cubic beziers.
    pub fn circle(r: f64) -> Contour {
        let k = 0.552_284_749_8 * r;
        Contour {
            closed: true,
            tool: Tool::Cut,
            start: (r, 0.0),
            segs: vec![
                Seg::Cubic { c1: (r, k), c2: (k, r), to: (0.0, r) },
                Seg::Cubic { c1: (-k, r), c2: (-r, k), to: (-r, 0.0) },
                Seg::Cubic { c1: (-r, -k), c2: (-k, -r), to: (0.0, -r) },
                Seg::Cubic { c1: (k, -r), c2: (r, -k), to: (r, 0.0) },
            ],
        }
    }
    fn translated(&self, dx: f64, dy: f64) -> Contour {
        let t = |p: (f64, f64)| (p.0 + dx, p.1 + dy);
        Contour {
            closed: self.closed,
            tool: self.tool,
            start: t(self.start),
            segs: self
                .segs
                .iter()
                .map(|s| match *s {
                    Seg::Line(p) => Seg::Line(t(p)),
                    Seg::Cubic { c1, c2, to } => Seg::Cubic { c1: t(c1), c2: t(c2), to: t(to) },
                })
                .collect(),
        }
    }
    fn points(&self) -> Vec<(f64, f64)> {
        let mut v = vec![self.start];
        for s in &self.segs {
            match *s {
                Seg::Line(p) => v.push(p),
                Seg::Cubic { c1, c2, to } => v.extend([c1, c2, to]),
            }
        }
        v
    }
}

/// One cut piece = one or more contours (e.g. a letter and its holes) placed on the mat.
pub struct PieceSpec {
    pub contours: Vec<Contour>,
    /// Where the piece origin sits on the mat, in mm from the top-left of the cut area.
    pub position_mm: (f64, f64),
}

pub struct Design {
    pub mat_width_mm: f64,
    pub mat_height_mm: f64,
    pub pieces: Vec<PieceSpec>,
}

impl Default for Design {
    fn default() -> Self {
        Design { mat_width_mm: 296.67, mat_height_mm: 298.80, pieces: vec![] }
    }
}

fn q(v: f64) -> i32 {
    (v * 100.0).round() as i32
}

impl Design {
    pub fn to_fcm(&self) -> Fcm {
        let mut pieces = Vec::new();
        for p in &self.pieces {
            // bbox of control points, in piece-local mm; recentre so the piece origin is its bbox centre
            let pts: Vec<_> = p.contours.iter().flat_map(|c| c.points()).collect();
            let (minx, maxx) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
            let (miny, maxy) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
            let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
            let paths = p
                .contours
                .iter()
                .map(|c| {
                    let c = c.translated(-cx, -cy);
                    let mut outlines: Vec<Outline> = Vec::new();
                    for s in &c.segs {
                        match *s {
                            Seg::Line(pt) => {
                                let seg = Segment::Line { x: q(pt.0), y: q(pt.1) };
                                match outlines.last_mut() {
                                    Some(o) if o.kind == 0 => o.segments.push(seg),
                                    _ => outlines.push(Outline { kind: 0, segments: vec![seg] }),
                                }
                            }
                            Seg::Cubic { c1, c2, to } => {
                                let seg = Segment::Bezier { c1x: q(c1.0), c1y: q(c1.1), c2x: q(c2.0), c2y: q(c2.1), x: q(to.0), y: q(to.1) };
                                match outlines.last_mut() {
                                    Some(o) if o.kind == 1 => o.segments.push(seg),
                                    _ => outlines.push(Outline { kind: 1, segments: vec![seg] }),
                                }
                            }
                        }
                    }
                    Path {
                        flags: c.tool.flags() | if c.closed { 0 } else { flags::OPEN },
                        kind: PathKind::Line { unk0: 0, unk1_f32_bits: 0.5f32.to_bits(), start_x: q(c.start.0), start_y: q(c.start.1), outlines },
                    }
                })
                .collect();
            pieces.push(Piece {
                unk0: 0,
                unk1: 0,
                width: q(maxx - minx) - 1,
                height: q(maxy - miny) - 1,
                unk4: 1,
                affine: [1.0, 0.0, 0.0, 1.0, (p.position_mm.0 * 100.0) as f32, (p.position_mm.1 * 100.0) as f32],
                unk11: 0,
                unk12: 0,
                unk13: 4,
                flags: [0; 4],
                paths,
            });
        }
        let thumb = self.thumbnail();
        Fcm {
            header: FileHeader {
                format: *b"#FCM",
                version: *b"0100",
                content_id: 0x17d7_8402,
                short_name: [0; 8],
                long_name: vec![0x20],
                author: vec![0x20],
                copyright: vec![0x20],
                thumb_block_w: 3,
                thumb_block_h: 3,
                thumbnail: thumb,
                machine_model_id: *b"1APP",
                save_machine_ver: 207,
            },
            cut: CutHeader {
                mat_id: 0,
                cut_area_width: (self.mat_width_mm * 100.0).round() as u32,
                cut_area_height: (self.mat_height_mm * 100.0).round() as u32,
                seam_allowance_width: 2000,
                align: None,
            },
            piece_labels: vec![0; pieces.len()],
            pieces,
            tail: vec![],
        }
    }

    /// 88x88 1-bit BMP (62-byte header, black/white palette, bottom-up rows padded to 12 bytes).
    fn thumbnail(&self) -> Vec<u8> {
        const N: usize = 88;
        let mut img = vec![[false; N]; N]; // true = ink
        let (mw, mh) = (self.mat_width_mm, self.mat_height_mm);
        let scale = (N as f64 - 1.0) / mw.max(mh);
        let to_px = |p: (f64, f64)| (p.0 * scale, p.1 * scale);
        for ps in &self.pieces {
            for c in &ps.contours {
                // centre contour on piece bbox like to_fcm does
                let all: Vec<_> = ps.contours.iter().flat_map(|c| c.points()).collect();
                let (minx, maxx) = all.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
                let (miny, maxy) = all.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
                let c = c.translated(ps.position_mm.0 - (minx + maxx) / 2.0, ps.position_mm.1 - (miny + maxy) / 2.0);
                let mut cur = c.start;
                for s in &c.segs {
                    let pts: Vec<(f64, f64)> = match *s {
                        Seg::Line(p) => vec![cur, p],
                        Seg::Cubic { c1, c2, to } => (0..=16)
                            .map(|i| {
                                let t = i as f64 / 16.0;
                                let u = 1.0 - t;
                                (
                                    u * u * u * cur.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * to.0,
                                    u * u * u * cur.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * to.1,
                                )
                            })
                            .collect(),
                    };
                    for w in pts.windows(2) {
                        let (Some(&p0), Some(&p1)) = (w.first(), w.get(1)) else { continue };
                        let (a, b) = (to_px(p0), to_px(p1));
                        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).clamp(1, 400) * 2;
                        for i in 0..=steps {
                            let t = i as f64 / steps as f64;
                            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                            if x >= 0.0 && y >= 0.0 && (x as usize) < N && (y as usize) < N {
                                img[y as usize][x as usize] = true;
                            }
                        }
                    }
                    cur = match *s {
                        Seg::Line(p) => p,
                        Seg::Cubic { to, .. } => to,
                    };
                }
            }
        }
        let row_bytes = 12usize;
        let size = 62 + row_bytes * N;
        let mut b = Vec::with_capacity(size);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(size as u32).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&62u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(N as i32).to_le_bytes());
        b.extend_from_slice(&(N as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&((row_bytes * N) as u32).to_le_bytes());
        b.extend_from_slice(&3780u32.to_le_bytes());
        b.extend_from_slice(&3780u32.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, 0xff, 0]); // palette: 0=black 1=white
        for y in (0..N).rev() {
            let mut row = vec![0u8; row_bytes];
            for x in 0..N {
                if !img[y][x] {
                    row[x / 8] |= 0x80 >> (x % 8); // white
                }
            }
            b.extend_from_slice(&row);
        }
        b
    }
}
