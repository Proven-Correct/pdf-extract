//! Generators for hostile and structurally unusual PDFs (spec §3.4, T50,
//! Appendix C).
//!
//! Each generator targets one resource bound of spec §2.3 and is the single
//! source of truth for its fixture: the committed corpus-C copies are small
//! parameterisations of these functions, and the large variants (200,000-deep
//! nesting, 1M-page trees, 50 MiB bombs) are generated at test time and never
//! committed.
//!
//! Apart from the hostility under test, every file is well formed: exact xref
//! offsets, correct `/Length`s, a one-page or explicit page tree. So a reader
//! that rejects or repairs one of these files is reacting to the intended
//! hazard, not to an incidental defect.
//!
//! Visible text is placed in each file so tests can tell "page extracted, the
//! over-deep object was dropped" from "page lost": see each function's docs
//! for the exact strings.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::pdfwriter::{ObjWriter, Objects, stream_body};

const HELV: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";
const MEDIA_BOX: &str = "[0 0 595 842]";

/// Number of leaves of [`TreeShape::LyingCount`] (the shape fixes the real
/// count so only the claimed `/Count` varies; spec §4.8 tests 100 leaves).
pub const LYING_COUNT_LEAVES: usize = 100;

/// Branching factor of [`big_tree_to_path`]: a balanced tree with 32 kids per
/// node keeps 1M pages at depth 4, like real writers' balanced trees.
pub const BIG_TREE_FANOUT: usize = 32;

/// Page-tree shapes for [`page_tree`]. Leaves show the text `Page <n>`
/// (n counted from 1 in document order); inherited `/MediaBox` and
/// `/Resources` sit on the root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeShape {
    /// Root with `n` leaf kids.
    Flat(usize),
    /// Balanced tree: `levels` levels of interior nodes below and including
    /// the root (levels >= 1), each with `leaves_per_node` kids, so
    /// `leaves_per_node ^ levels` leaves. `Fanout { 100, 2 }` is 10,000
    /// leaves under 100 interior nodes under the root.
    Fanout {
        /// Kids per interior node.
        leaves_per_node: usize,
        /// Interior levels (root included).
        levels: u32,
    },
    /// Root `/Kids [leaf root]` with `/Count 2`: the root lists itself.
    Cycle,
    /// Root `/Kids [leaf leaf]` (same reference twice), `/Count 2`.
    DuplicateLeaf,
    /// [`LYING_COUNT_LEAVES`] leaves under a root whose `/Count` is `claimed`.
    LyingCount {
        /// The `/Count` written on the root.
        claimed: usize,
    },
    /// Two leaves, the second without `/Type`; `/Count 2`.
    TypelessLeaf,
    /// Two leaves, the second a direct dictionary in `/Kids` rather than a
    /// reference; `/Count 2`.
    DirectDictKid,
}

/// `[` x depth then `]` x depth: one array nested `depth` levels deep.
fn nested_array(depth: usize) -> String {
    let mut s = String::with_capacity(depth * 2);
    s.extend(std::iter::repeat_n('[', depth));
    s.extend(std::iter::repeat_n(']', depth));
    s
}

/// Assembles a one-page document around existing objects. `resources` is a
/// direct dictionary or an indirect reference, written verbatim.
fn one_page(
    mut objs: Objects,
    resources: &str,
    contents: Option<u32>,
    page_extra: &str,
) -> Vec<u8> {
    let catalog = objs.alloc();
    let root = objs.alloc();
    let page = objs.alloc();
    let contents = contents.map_or_else(String::new, |c| format!(" /Contents {c} 0 R"));
    objs.set(
        page,
        format!(
            "<< /Type /Page /Parent {root} 0 R /MediaBox {MEDIA_BOX} /Resources {resources}{contents}{page_extra} >>"
        ),
    );
    objs.set(
        root,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    objs.set(catalog, format!("<< /Type /Catalog /Pages {root} 0 R >>"));
    objs.to_bytes(catalog, "")
}

fn helv_resources(objs: &mut Objects) -> String {
    let font = objs.add(HELV);
    format!("<< /Font << /Helv {font} 0 R >> >>")
}

/// A page whose dictionary carries `/Deep` = an array nested `depth` levels.
/// Content shows `Deep`; a reader that cannot parse the page dictionary loses
/// the whole page's content (the oracle then yields `""`).
pub fn deep_nesting_in_page_dict(depth: usize) -> Vec<u8> {
    let mut objs = Objects::new();
    let res = helv_resources(&mut objs);
    let c = objs.add(stream_body("", b"BT /Helv 11 Tf 72 720 Td (Deep) Tj ET"));
    one_page(
        objs,
        &res,
        Some(c),
        &format!(" /Deep {}", nested_array(depth)),
    )
}

/// Content `(Before) Tj <array nested depth levels> TJ (After) Tj`: only the
/// `TJ` operand is over-deep. `Before` survives any reader that drops it;
/// whether `After` does depends on where the reader resynchronises (the
/// oracle loses it: `20_000` and `200_000` both extract as `Before`).
pub fn deep_nesting_in_content(depth: usize) -> Vec<u8> {
    let mut objs = Objects::new();
    let res = helv_resources(&mut objs);
    let content = format!(
        "BT /Helv 11 Tf 72 720 Td (Before) Tj {} TJ (After) Tj ET",
        nested_array(depth)
    );
    let c = objs.add(stream_body("", content.as_bytes()));
    one_page(objs, &res, Some(c), "")
}

/// The page's `/Resources` (an indirect object) carries `/Deep` nested
/// `depth` levels next to `/Font`. Content shows `Res`; a reader that drops
/// the resources dictionary must fall back for the font.
pub fn deep_nesting_in_resources(depth: usize) -> Vec<u8> {
    let mut objs = Objects::new();
    let font = objs.add(HELV);
    let res = objs.add(format!(
        "<< /Font << /Helv {font} 0 R >> /Deep {} >>",
        nested_array(depth)
    ));
    let c = objs.add(stream_body("", b"BT /Helv 11 Tf 72 720 Td (Res) Tj ET"));
    one_page(objs, &format!("{res} 0 R"), Some(c), "")
}

/// A Separation colour space whose tint transform is a Type 4 (PostScript
/// calculator) function with procedures nested `depth` levels (counting the
/// outer braces). The program is valid at any depth: it is
/// `{ pop 0 0 0 true { true { ... } if } if }`, leaving three zeros. The page
/// fills a rectangle in that colour space, then shows `Fn`.
pub fn deep_nesting_in_type4_function(depth: usize) -> Vec<u8> {
    let depth = depth.max(1);
    let mut program = String::with_capacity(depth * 12 + 16);
    program.push_str("{ pop 0 0 0");
    for _ in 1..depth {
        program.push_str(" true {");
    }
    for _ in 1..depth {
        program.push_str(" } if");
    }
    program.push_str(" }");
    let mut objs = Objects::new();
    let font = objs.add(HELV);
    let func = objs.add(stream_body(
        "/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]",
        program.as_bytes(),
    ));
    let res = format!(
        "<< /Font << /Helv {font} 0 R >> /ColorSpace << /CS0 [/Separation /Spot /DeviceRGB {func} 0 R] >> >>"
    );
    let c = objs.add(stream_body(
        "",
        b"/CS0 cs 0.5 scn 72 600 100 100 re f 0 g BT /Helv 11 Tf 72 720 Td (Fn) Tj ET",
    ));
    one_page(objs, &res, Some(c), "")
}

/// A page tree that is a single chain: the root, then `depth - 1` further
/// `/Pages` nodes each with one kid, then one leaf showing `DeepLeaf`. The
/// leaf sits at depth `depth` (root = depth 0). `depth == 0` gives a root
/// with no kids.
pub fn deep_page_tree(depth: usize) -> Vec<u8> {
    let mut objs = Objects::new();
    let font = objs.add(HELV);
    let catalog = objs.alloc();
    let nodes: Vec<u32> = (0..depth.max(1)).map(|_| objs.alloc()).collect();
    let root = nodes[0];
    let c = objs.add(stream_body(
        "",
        b"BT /Helv 11 Tf 72 720 Td (DeepLeaf) Tj ET",
    ));
    let inherit = format!("/MediaBox {MEDIA_BOX} /Resources << /Font << /Helv {font} 0 R >> >>");
    if depth == 0 {
        objs.set(
            root,
            format!("<< /Type /Pages /Kids [] /Count 0 {inherit} >>"),
        );
    } else {
        let leaf = objs.alloc();
        let last = *nodes.last().expect("depth >= 1 nodes");
        objs.set(
            leaf,
            format!("<< /Type /Page /Parent {last} 0 R /Contents {c} 0 R >>"),
        );
        for (i, &node) in nodes.iter().enumerate() {
            let kid = nodes.get(i + 1).copied().unwrap_or(leaf);
            let parent = if i == 0 {
                inherit.clone()
            } else {
                format!("/Parent {} 0 R", nodes[i - 1])
            };
            objs.set(
                node,
                format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 {parent} >>"),
            );
        }
    }
    objs.set(catalog, format!("<< /Type /Catalog /Pages {root} 0 R >>"));
    objs.to_bytes(catalog, "")
}

/// Shared state while building a [`page_tree`].
struct TreeBuild {
    objs: Objects,
    next_page: usize,
}

impl TreeBuild {
    /// Writes a leaf (content + dictionary) under `parent`; returns its id.
    fn leaf(&mut self, parent: u32, typed: bool) -> u32 {
        self.next_page += 1;
        let content = format!("BT /Helv 11 Tf 72 720 Td (Page {}) Tj ET", self.next_page);
        let c = self.objs.add(stream_body("", content.as_bytes()));
        let ty = if typed { "/Type /Page " } else { "" };
        self.objs
            .add(format!("<< {ty}/Parent {parent} 0 R /Contents {c} 0 R >>"))
    }

    /// Interior node at `id` with `levels` levels below-and-including it.
    fn fanout(
        &mut self,
        id: u32,
        parent: Option<u32>,
        per: usize,
        levels: u32,
        extra: &str,
    ) -> usize {
        let mut kids = Vec::with_capacity(per);
        let mut count = 0;
        for _ in 0..per {
            if levels <= 1 {
                kids.push(self.leaf(id, true));
                count += 1;
            } else {
                let kid = self.objs.alloc();
                count += self.fanout(kid, Some(id), per, levels - 1, "");
                kids.push(kid);
            }
        }
        let parent = parent.map_or_else(String::new, |p| format!(" /Parent {p} 0 R"));
        self.objs.set(
            id,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {count}{parent}{extra} >>",
                refs(&kids)
            ),
        );
        count
    }
}

fn refs(ids: &[u32]) -> String {
    let v: Vec<String> = ids.iter().map(|i| format!("{i} 0 R")).collect();
    v.join(" ")
}

/// A document whose page tree has the given shape (see [`TreeShape`]).
pub fn page_tree(shape: TreeShape) -> Vec<u8> {
    let mut t = TreeBuild {
        objs: Objects::new(),
        next_page: 0,
    };
    let font = t.objs.add(HELV);
    let catalog = t.objs.alloc();
    let root = t.objs.alloc();
    let inherit = format!(" /MediaBox {MEDIA_BOX} /Resources << /Font << /Helv {font} 0 R >> >>");
    let root_dict = |kids: &str, count: usize| {
        format!("<< /Type /Pages /Kids [{kids}] /Count {count}{inherit} >>")
    };
    match shape {
        TreeShape::Flat(n) => {
            let leaves: Vec<u32> = (0..n).map(|_| t.leaf(root, true)).collect();
            t.objs.set(root, root_dict(&refs(&leaves), n));
        }
        TreeShape::Fanout {
            leaves_per_node,
            levels,
        } => {
            t.fanout(root, None, leaves_per_node, levels.max(1), &inherit);
        }
        TreeShape::Cycle => {
            let leaf = t.leaf(root, true);
            t.objs
                .set(root, root_dict(&format!("{leaf} 0 R {root} 0 R"), 2));
        }
        TreeShape::DuplicateLeaf => {
            let leaf = t.leaf(root, true);
            t.objs
                .set(root, root_dict(&format!("{leaf} 0 R {leaf} 0 R"), 2));
        }
        TreeShape::LyingCount { claimed } => {
            let leaves: Vec<u32> = (0..LYING_COUNT_LEAVES)
                .map(|_| t.leaf(root, true))
                .collect();
            t.objs.set(root, root_dict(&refs(&leaves), claimed));
        }
        TreeShape::TypelessLeaf => {
            let a = t.leaf(root, true);
            let b = t.leaf(root, false);
            t.objs.set(root, root_dict(&format!("{a} 0 R {b} 0 R"), 2));
        }
        TreeShape::DirectDictKid => {
            let a = t.leaf(root, true);
            let c = t
                .objs
                .add(stream_body("", b"BT /Helv 11 Tf 72 720 Td (Page 2) Tj ET"));
            let direct = format!("<< /Type /Page /Parent {root} 0 R /Contents {c} 0 R >>");
            t.objs.set(root, root_dict(&format!("{a} 0 R {direct}"), 2));
        }
    }
    t.objs
        .set(catalog, format!("<< /Type /Catalog /Pages {root} 0 R >>"));
    t.objs.to_bytes(catalog, "")
}

/// Zlib-compresses `count` repetitions of `unit` (plus `prefix`/`suffix`)
/// without materialising the decoded data, which can be tens of megabytes.
fn deflate_repeated(prefix: &[u8], unit: &[u8], count: u64, suffix: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    // A chunk of whole units keeps the write loop short for large counts.
    let per_chunk = (64 * 1024 / unit.len().max(1)).max(1) as u64;
    let chunk = unit.repeat(per_chunk as usize);
    // Writes into a Vec cannot fail.
    enc.write_all(prefix).expect("Vec write is infallible");
    let mut left = count;
    while left >= per_chunk {
        enc.write_all(&chunk).expect("Vec write is infallible");
        left -= per_chunk;
    }
    for _ in 0..left {
        enc.write_all(unit).expect("Vec write is infallible");
    }
    enc.write_all(suffix).expect("Vec write is infallible");
    enc.finish().expect("Vec write is infallible")
}

/// One page whose content stream is `/FlateDecode` and inflates to exactly
/// `decoded_len` bytes of spaces: valid, empty content (page text `""`) that
/// costs `decoded_len` bytes to decode.
pub fn flate_bomb(decoded_len: u64) -> Vec<u8> {
    let mut objs = Objects::new();
    let res = helv_resources(&mut objs);
    let data = deflate_repeated(b"", b" ", decoded_len, b"");
    let c = objs.add(stream_body("/Filter /FlateDecode", &data));
    one_page(objs, &res, Some(c), "")
}

/// One page whose (flate-compressed) content is a `Start` text object, then
/// `ops` no-op path operators (`n`), then an `End` text object: `ops + 10`
/// operators in all. With an operator budget below `ops`, `Start` is emitted
/// and `End` is not.
pub fn operator_flood(ops: u64) -> Vec<u8> {
    let mut objs = Objects::new();
    let res = helv_resources(&mut objs);
    let data = deflate_repeated(
        b"BT /Helv 11 Tf 72 720 Td (Start) Tj ET\n",
        b"n\n",
        ops,
        b"BT /Helv 11 Tf 72 700 Td (End) Tj ET\n",
    );
    let c = objs.add(stream_body("/Filter /FlateDecode", &data));
    one_page(objs, &res, Some(c), "")
}

/// Form XObjects `F0..F<depth>`: each `Fi` draws `F(i+1)` twice and
/// `F<depth>` shows `Leaf`, so the page (which draws `F0` once) shows `Leaf`
/// 2^depth times at the same spot. Interior forms cost 2 operators per draw,
/// leaves 4, so the page costs 2·(2^depth − 1) + 4·2^depth + 4 operators
/// (about 25.2M at depth 22; spec §4.6's "≈12.6M" assumes one-operator leaves,
/// which would need a `Tj` outside a text object).
pub fn form_fanout(depth: u32) -> Vec<u8> {
    let mut objs = Objects::new();
    let font = objs.add(HELV);
    let ids: Vec<u32> = (0..=depth).map(|_| objs.alloc()).collect();
    for (i, &id) in ids.iter().enumerate() {
        let (content, xobj) = match ids.get(i + 1) {
            Some(next) => (
                format!("/F{} Do /F{} Do", i + 1, i + 1),
                format!(" /XObject << /F{} {next} 0 R >>", i + 1),
            ),
            None => ("BT /Helv 11 Tf (Leaf) Tj ET".to_owned(), String::new()),
        };
        objs.set(
            id,
            form(
                &content,
                &format!("<< /Font << /Helv {font} 0 R >>{xobj} >>"),
            ),
        );
    }
    let c = objs.add(stream_body("", b"q 1 0 0 1 72 720 cm /F0 Do Q"));
    let res = format!(
        "<< /Font << /Helv {font} 0 R >> /XObject << /F0 {} 0 R >> >>",
        ids[0]
    );
    one_page(objs, &res, Some(c), "")
}

/// A Form XObject stream. The `/BBox` is deliberately much larger than the
/// page: a form's bbox clips everything it draws, nested forms included, so a
/// page-sized bbox would clip the lower levels of [`nested_forms`] (each is
/// drawn 12 pt below its parent) and hide them from text extraction.
fn form(content: &str, resources: &str) -> Vec<u8> {
    stream_body(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [-2000 -2000 2000 2000] /Resources {resources}"
        ),
        content.as_bytes(),
    )
}

/// A Form XObject `F` that shows `Rec` and then draws itself. Readers must
/// stop it somehow (the oracle shows `Rec` once; unpatched hayro recurses to
/// its nesting cap).
pub fn form_self_recursion() -> Vec<u8> {
    let mut objs = Objects::new();
    let font = objs.add(HELV);
    let f = objs.alloc();
    objs.set(
        f,
        form(
            "BT /Helv 11 Tf 0 0 Td (Rec) Tj ET /F Do",
            &format!("<< /Font << /Helv {font} 0 R >> /XObject << /F {f} 0 R >> >>"),
        ),
    );
    let c = objs.add(stream_body("", b"q 1 0 0 1 72 720 cm /F Do Q"));
    let res = format!("<< /Font << /Helv {font} 0 R >> /XObject << /F {f} 0 R >> >>");
    one_page(objs, &res, Some(c), "")
}

/// A linear chain of `depth` Form XObjects `F1..F<depth>`: `Fi` shows `L<i>`
/// on its own line (12 pt below its parent's) and draws `F(i+1)`. The page
/// draws `F1`, so the nesting depth of `Fi` is `i`. `depth == 0` gives an
/// empty page.
pub fn nested_forms(depth: u32) -> Vec<u8> {
    let mut objs = Objects::new();
    let font = objs.add(HELV);
    let ids: Vec<u32> = (0..depth).map(|_| objs.alloc()).collect();
    for (i, &id) in ids.iter().enumerate() {
        let level = i + 1;
        let mut content = format!("BT /Helv 11 Tf 0 0 Td (L{level}) Tj ET");
        let mut res = format!("<< /Font << /Helv {font} 0 R >>");
        if let Some(next) = ids.get(i + 1) {
            let _ = write!(content, " q 1 0 0 1 0 -12 cm /F{} Do Q", level + 1);
            let _ = write!(res, " /XObject << /F{} {next} 0 R >>", level + 1);
        }
        res.push_str(" >>");
        objs.set(id, form(&content, &res));
    }
    let (content, res) = match ids.first() {
        Some(first) => (
            "q 1 0 0 1 72 780 cm /F1 Do Q".to_owned(),
            format!("<< /Font << /Helv {font} 0 R >> /XObject << /F1 {first} 0 R >> >>"),
        ),
        None => (String::new(), format!("<< /Font << /Helv {font} 0 R >> >>")),
    };
    let c = objs.add(stream_body("", content.as_bytes()));
    one_page(objs, &res, Some(c), "")
}

/// Streams a document with `leaves` pages in a balanced tree
/// ([`BIG_TREE_FANOUT`] kids per node) to `path`. All leaves share one
/// content stream (`Leaf`), so 1M pages is about 108 MB rather than several
/// times that; memory use is one offset per object.
///
/// # Errors
/// Any I/O error creating or writing the file.
///
/// # Panics
/// Beyond 2^32 objects (about 4 billion leaves), which PDF object numbers
/// cannot express anyway.
pub fn big_tree_to_path(leaves: usize, path: &Path) -> io::Result<()> {
    const F: usize = BIG_TREE_FANOUT;
    // Level sizes, leaves first; at least one interior level (the root).
    let mut sizes = vec![leaves];
    loop {
        let below = *sizes.last().expect("non-empty");
        let n = below.div_ceil(F).max(1);
        sizes.push(n);
        if n == 1 {
            break;
        }
    }
    // Object numbers: 1 catalog, 2 font, 3 shared content, then each level.
    let mut bases = Vec::with_capacity(sizes.len());
    let mut next = 4usize;
    for &n in &sizes {
        bases.push(next);
        next += n;
    }
    let id = |level: usize, j: usize| -> u32 {
        u32::try_from(bases[level] + j).expect("fewer than 2^32 objects")
    };
    let top = sizes.len() - 1;
    let root = id(top, 0);

    let mut w = ObjWriter::new(BufWriter::new(File::create(path)?))?;
    w.object(
        1,
        format!("<< /Type /Catalog /Pages {root} 0 R >>").as_bytes(),
    )?;
    w.object(2, HELV.as_bytes())?;
    w.object(
        3,
        &stream_body("", b"BT /Helv 11 Tf 72 720 Td (Leaf) Tj ET"),
    )?;
    for j in 0..leaves {
        let parent = id(1, j / F);
        w.object(
            id(0, j),
            format!("<< /Type /Page /Parent {parent} 0 R /Contents 3 0 R >>").as_bytes(),
        )?;
    }
    let mut span = 1usize; // leaves under one node of the level below
    for level in 1..=top {
        let child_span = span;
        span = span.saturating_mul(F);
        for j in 0..sizes[level] {
            let first = j * F;
            let last = (first + F).min(sizes[level - 1]);
            let kids: Vec<u32> = (first..last).map(|k| id(level - 1, k)).collect();
            let count = (last * child_span).min(leaves) - (first * child_span).min(leaves);
            let mut body = format!("<< /Type /Pages /Kids [{}] /Count {count}", refs(&kids));
            if level == top {
                let _ = write!(
                    body,
                    " /MediaBox {MEDIA_BOX} /Resources << /Font << /Helv 2 0 R >> >>"
                );
            } else {
                let _ = write!(body, " /Parent {} 0 R", id(level + 1, j / F));
            }
            body.push_str(" >>");
            w.object(id(level, j), body.as_bytes())?;
        }
    }
    let mut out = w.finish(1, "")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|&b| if b < 0x80 { b as char } else { '?' })
            .collect()
    }

    #[test]
    fn nested_array_shape() {
        assert_eq!(nested_array(3), "[[[]]]");
        assert_eq!(nested_array(0), "");
    }

    #[test]
    fn type4_program_nesting_counts_outer_braces() {
        let s = text(&deep_nesting_in_type4_function(3));
        assert!(s.contains("{ pop 0 0 0 true { true { } if } if }"));
        let s1 = text(&deep_nesting_in_type4_function(1));
        assert!(s1.contains("{ pop 0 0 0 }"));
    }

    #[test]
    fn deep_page_tree_chain_length() {
        let s = text(&deep_page_tree(5));
        assert_eq!(s.matches("/Type /Pages").count(), 5);
        assert_eq!(s.matches("/Type /Page ").count(), 1);
        let empty = text(&deep_page_tree(0));
        assert!(empty.contains("/Kids [] /Count 0"));
    }

    #[test]
    fn flood_and_bomb_decode_to_the_stated_sizes() {
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let inflate = |pdf: &[u8]| -> Vec<u8> {
            let s = text(pdf);
            let start = s.find("stream\n").expect("stream") + 7;
            let end = s.rfind("\nendstream").expect("endstream");
            let mut out = Vec::new();
            ZlibDecoder::new(&pdf[start..end])
                .read_to_end(&mut out)
                .expect("valid zlib");
            out
        };
        let bomb = inflate(&flate_bomb(200_000));
        assert_eq!(bomb.len(), 200_000);
        assert!(bomb.iter().all(|&b| b == b' '));
        let flood = inflate(&operator_flood(70_000));
        let flood = String::from_utf8(flood).expect("ASCII");
        assert_eq!(flood.lines().filter(|l| *l == "n").count(), 70_000);
        assert!(flood.starts_with("BT /Helv 11 Tf 72 720 Td (Start) Tj ET\n"));
        assert!(flood.ends_with("(End) Tj ET\n"));
    }

    #[test]
    fn fanout_counts() {
        let s = text(&page_tree(TreeShape::Fanout {
            leaves_per_node: 3,
            levels: 2,
        }));
        assert_eq!(s.matches("/Type /Page ").count(), 9);
        assert!(s.contains("/Count 9 /MediaBox"));
        assert_eq!(s.matches("/Count 3 /Parent").count(), 3);
    }
}
