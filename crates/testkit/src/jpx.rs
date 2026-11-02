//! Hand-assembled JPEG 2000 codestreams (ISO/IEC 15444-1 Annex A) for the
//! decoder memory-budget tests (run 0003 H15, oracle §4.5), and PDFs that
//! draw one.
//!
//! The shapes under test are hostile geometries (1 × 1 tiles, 1 × 1
//! code-blocks, a reference grid 255 times larger than the image, hundreds
//! of components) that the decoder sizes from the main header alone, so
//! they need no entropy-coded data: [`codestream`] writes the main header
//! (`SIZ`, `COD`, `QCD`) and one tile-part per tile whose body is empty
//! (`SOT` then `SOD`, no packet bytes). hayro-jpeg2000 reads an empty body
//! as "no packets" outside strict mode, so such a codestream decodes, to a
//! flat mid-gray image, after building every record its header declares;
//! that is what lets the tests compare a budgeted decode with an unbudgeted
//! one on the very shapes the budget charges. Real image content comes from
//! [`gray_64`], a 64 × 64 codestream written once by an external encoder,
//! whose `SIZ` [`patched_gray_64`] rewrites for the refusal tests (the body
//! then no longer matches, which does not matter to a refusal from the
//! header).

use crate::pdfwriter::{Objects, stream_body};

/// Marker codes (Table A.2).
mod marker {
    pub(super) const SOC: u16 = 0xFF4F;
    pub(super) const SIZ: u16 = 0xFF51;
    pub(super) const COD: u16 = 0xFF52;
    pub(super) const QCD: u16 = 0xFF5C;
    pub(super) const SOT: u16 = 0xFF90;
    pub(super) const SOD: u16 = 0xFF93;
    pub(super) const EOC: u16 = 0xFFD9;
}

/// A 64 × 64 gray JPEG 2000 codestream with six resolution levels (five
/// decompositions), one 64 × 64 tile, reversible 5/3 wavelet: written by
/// OpenJPEG through Pillow (`save(..., "JPEG2000", no_jp2=True,
/// num_resolutions=6, irreversible=False)`) with its comment marker removed.
/// The same bytes as the H13 fixture of `crates/core/tests/h13_image_pixels.rs`.
const GRAY_64: [&str; 3] = [
    "ff4fff510029000000000040000000400000000000000000000000400000004000000000000000000001070101ff5200",
    "0c00000001000504040001ff5c00134040484850484850484850484850484850ff90000a0000000000290001ff93df18",
    "40078b735dc7c0180d9c4edf184024b516ebdc183951488080ffd9",
];

fn unhex(parts: &[&str]) -> Vec<u8> {
    let hex: String = parts.concat();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A 64 × 64 gray codestream written by OpenJPEG (six resolution levels, one
/// tile, reversible 5/3 wavelet): a real image that decodes to non-uniform
/// pixels.
pub fn gray_64() -> Vec<u8> {
    unhex(&GRAY_64)
}

/// [`gray_64`] with its `SIZ` rewritten: a `grid.0 × grid.1` reference grid
/// (`Xsiz`, `Ysiz`; offsets stay 0), `tile.0 × tile.1` tiles (`XTsiz`,
/// `YTsiz`) and the one component subsampled by `subsampling` both ways
/// (`XRsiz`, `YRsiz`), so the image is `ceil(grid / subsampling)` pixels a
/// side. The body still codes one 64 × 64 tile: only refusals from the
/// header are meaningful.
pub fn patched_gray_64(grid: (u32, u32), tile: (u32, u32), subsampling: u8) -> Vec<u8> {
    let mut data = gray_64();
    // SOC, SIZ, Lsiz = 41: one component.
    assert_eq!(&data[..6], [0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29], "SOC, SIZ");
    data[8..12].copy_from_slice(&grid.0.to_be_bytes());
    data[12..16].copy_from_slice(&grid.1.to_be_bytes());
    data[24..28].copy_from_slice(&tile.0.to_be_bytes());
    data[28..32].copy_from_slice(&tile.1.to_be_bytes());
    data[43] = subsampling;
    data[44] = subsampling;
    data
}

/// The geometry of a [`codestream`]. Built with [`Shape::new`] and adjusted
/// field by field.
#[derive(Clone, Debug)]
pub struct Shape {
    /// The reference grid, `Xsiz × Ysiz` (image and tile offsets are 0).
    pub grid: (u32, u32),
    /// The tile size, `XTsiz × YTsiz`.
    pub tile: (u32, u32),
    /// `XRsiz = YRsiz` of every component.
    pub subsampling: u8,
    /// `Csiz`: 8-bit unsigned components.
    pub components: u16,
    /// `NL`: wavelet decompositions (resolution levels minus one).
    pub decompositions: u8,
    /// The precinct exponent `PPx = PPy` at every resolution, or `None` for
    /// the default (2¹⁵, no precinct partition).
    pub precinct_exponent: Option<u8>,
    /// Quality layers.
    pub layers: u16,
    /// The progression order (Table A.16, 0 = LRCP … 4 = CPRL).
    pub progression: u8,
    /// Whether to write one empty tile-part per tile (up to the 65,536
    /// tiles a tile-part can address); without them the codestream is only
    /// good for a refusal from the header.
    pub tile_parts: bool,
}

impl Shape {
    /// One `width × height` tile of one component, no decompositions, no
    /// precincts, one layer, LRCP, with its tile-part.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            grid: (width, height),
            tile: (width, height),
            subsampling: 1,
            components: 1,
            decompositions: 0,
            precinct_exponent: None,
            layers: 1,
            progression: 0,
            tile_parts: true,
        }
    }

    /// The number of tiles, `ceil(Xsiz / XTsiz) × ceil(Ysiz / YTsiz)`.
    pub fn tiles(&self) -> u64 {
        u64::from(self.grid.0.div_ceil(self.tile.0)) * u64::from(self.grid.1.div_ceil(self.tile.1))
    }

    /// The image size the decoder reports: the grid shrunk by the
    /// subsampling.
    pub fn image_size(&self) -> (u32, u32) {
        let s = u32::from(self.subsampling);
        (self.grid.0.div_ceil(s), self.grid.1.div_ceil(s))
    }
}

fn put_marker(out: &mut Vec<u8>, code: u16, body: &[u8]) {
    out.extend_from_slice(&code.to_be_bytes());
    let len = u16::try_from(body.len() + 2).expect("marker segment fits");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(body);
}

/// A raw codestream (`SOC` … `EOC`) of `shape`, reversible 5/3 wavelet, no
/// quantization, whose tile-parts carry no packets.
pub fn codestream(shape: &Shape) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&marker::SOC.to_be_bytes());

    // SIZ (A.5.1).
    let mut siz = Vec::new();
    siz.extend_from_slice(&0u16.to_be_bytes()); // Rsiz
    for v in [
        shape.grid.0,
        shape.grid.1,
        0,
        0,
        shape.tile.0,
        shape.tile.1,
        0,
        0,
    ] {
        siz.extend_from_slice(&v.to_be_bytes());
    }
    siz.extend_from_slice(&shape.components.to_be_bytes());
    for _ in 0..shape.components {
        siz.extend_from_slice(&[7, shape.subsampling, shape.subsampling]);
    }
    put_marker(&mut out, marker::SIZ, &siz);

    // COD (A.6.1): Scod bit 0 announces the precinct sizes; 64 × 64
    // code-blocks (exponent 4 + 2), no style flags, 5/3 wavelet.
    let mut cod = vec![
        u8::from(shape.precinct_exponent.is_some()),
        shape.progression,
    ];
    cod.extend_from_slice(&shape.layers.to_be_bytes());
    cod.extend_from_slice(&[0, shape.decompositions, 4, 4, 0, 1]);
    if let Some(e) = shape.precinct_exponent {
        cod.extend(std::iter::repeat_n(
            e << 4 | e,
            usize::from(shape.decompositions) + 1,
        ));
    }
    put_marker(&mut out, marker::COD, &cod);

    // QCD (A.6.4): no quantization, two guard bits, one exponent per
    // sub-band (LL, then HL, LH, HH per decomposition).
    let mut qcd = vec![2 << 5];
    qcd.push(8 << 3);
    for _ in 0..shape.decompositions {
        qcd.extend_from_slice(&[9 << 3, 9 << 3, 10 << 3]);
    }
    put_marker(&mut out, marker::QCD, &qcd);

    if shape.tile_parts {
        let tiles = u16::try_from(shape.tiles() - 1).expect("tile-parts address 65,536 tiles");
        for index in 0..=tiles {
            // SOT (A.4.2): Psot covers SOT's 12 bytes and SOD's 2.
            let mut sot = index.to_be_bytes().to_vec();
            sot.extend_from_slice(&14u32.to_be_bytes());
            sot.extend_from_slice(&[0, 1]);
            put_marker(&mut out, marker::SOT, &sot);
            out.extend_from_slice(&marker::SOD.to_be_bytes());
        }
    }

    out.extend_from_slice(&marker::EOC.to_be_bytes());
    out
}

fn jp2_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let len = u32::try_from(body.len() + 8).expect("box fits");
    let mut out = len.to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A JP2 file (Annex I) whose header has a palette (`pclr`, one 8-bit
/// column, the identity over 256 entries) and a component mapping through
/// it, around a one-tile `side × side` codestream of `components`
/// components. Every index is the mid-gray 128 (the tiles carry no
/// packets), so the palette must hold it: the identity also makes resolving
/// the indices give what leaving them unresolved does (internal #77). With a
/// palette present hayro-jpeg2000 skips its check of the component count
/// against the colour space; with indices unresolved, the H16 patch refuses
/// any count but one (and, either way, any count above 255) with
/// `TooManyChannels` (internal #63).
pub fn jp2_palette(components: u16, side: u32) -> Vec<u8> {
    jp2_palette_mapped(components, side, |i| i)
}

/// [`jp2_palette`] with entry `i` of the palette holding `entry(i)` instead
/// of `i`. A mapping that moves 128 makes resolved indices tell apart from
/// unresolved ones, which the identity cannot.
pub fn jp2_palette_mapped(components: u16, side: u32, entry: impl Fn(u8) -> u8) -> Vec<u8> {
    let shape = Shape {
        components,
        ..Shape::new(side, side)
    };
    let mut header = Vec::new();
    // ihdr (I.5.3.1): height, width, components, 8-bit, JPEG 2000, no IPR.
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&side.to_be_bytes());
    ihdr.extend_from_slice(&side.to_be_bytes());
    ihdr.extend_from_slice(&components.to_be_bytes());
    ihdr.extend_from_slice(&[7, 7, 0, 0]);
    header.extend(jp2_box(b"ihdr", &ihdr));
    // colr: enumerated greyscale (17).
    header.extend(jp2_box(b"colr", &[1, 0, 0, 0, 0, 0, 17]));
    // pclr: 256 entries, 1 column of 8 bits: entry i is `entry(i)`.
    let mut pclr = vec![1, 0, 1, 7];
    pclr.extend((0..=u8::MAX).map(entry));
    header.extend(jp2_box(b"pclr", &pclr));
    // cmap: component 0 through palette column 0.
    header.extend(jp2_box(b"cmap", &[0, 0, 1, 0]));

    let mut out = Vec::new();
    out.extend(jp2_box(b"jP  ", &[0x0D, 0x0A, 0x87, 0x0A]));
    out.extend(jp2_box(b"ftyp", b"jp2 \0\0\0\0jp2 "));
    out.extend(jp2_box(b"jp2h", &header));
    out.extend(jp2_box(b"jp2c", &codestream(&shape)));
    out
}

/// A one-page PDF (200 × 200 pt) drawing `jpx` as a `width × height`
/// 8-bit gray image XObject (`/Filter /JPXDecode`) over the whole page.
pub fn image_pdf(jpx: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut objs = Objects::new();
    let image = objs.add(stream_body(
        &format!(
            "/Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /JPXDecode"
        ),
        jpx,
    ));
    let content = objs.add(stream_body("", b"q 200 0 0 200 0 0 cm /Im1 Do Q"));
    let catalog = objs.alloc();
    let root = objs.alloc();
    let page = objs.alloc();
    objs.set(
        page,
        format!(
            "<< /Type /Page /Parent {root} 0 R /MediaBox [0 0 200 200] \
             /Resources << /XObject << /Im1 {image} 0 R >> >> /Contents {content} 0 R >>"
        ),
    );
    objs.set(
        root,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    objs.set(catalog, format!("<< /Type /Catalog /Pages {root} 0 R >>"));
    objs.to_bytes(catalog, "")
}

/// The reference grid of [`tile_bomb`]: 60,180 a side, which subsampling by
/// 255 shrinks to a 236 × 236 image.
pub const TILE_BOMB_GRID: u32 = 60_180;

/// The hostile codestream of run 0003 §4.5: [`gray_64`] patched to 1 × 1
/// tiles over a 60,180-square reference grid subsampled by 255. The image
/// is 236 × 236 pixels, so the pixel cap passes it, but the grid holds
/// 3.6 billion tiles (just under the `u32` the tile count is kept in), each
/// with a tile record of over a hundred bytes: hundreds of gigabytes, from a
/// header refused under any memory budget.
pub fn tile_bomb() -> Vec<u8> {
    patched_gray_64((TILE_BOMB_GRID, TILE_BOMB_GRID), (1, 1), 255)
}

/// [`tile_bomb`] as a 236 × 236 image XObject. Under any memory budget the
/// image is skipped with an `image: memory limit` warning and the page
/// renders blank; unbudgeted, decoding it would allocate hundreds of
/// gigabytes of tile records.
pub fn tile_bomb_pdf() -> Vec<u8> {
    let side = TILE_BOMB_GRID.div_ceil(255);
    image_pdf(&tile_bomb(), side, side)
}
