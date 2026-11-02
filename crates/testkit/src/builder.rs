//! `PdfBuilder` / `PageBuilder`: well-formed test documents (spec §3.4, T49,
//! T50).
//!
//! Numbers other than `insert_text`'s are not validated: a NaN or infinite
//! page size, rectangle or matrix is a test-authoring error and panics at
//! [`PdfBuilder::to_bytes`] rather than producing an unparseable file.
//!
//! Model: a document is a list of pages; each page accumulates one content
//! stream (text runs and raw operators, in call order) plus the resources the
//! page declares. Nothing is serialised until [`PdfBuilder::to_bytes`], which
//! numbers objects deterministically, so the same calls always produce the
//! same bytes (the Python test kit is later checked against this byte for
//! byte, and fixture SHA-256s depend on it).
//!
//! Layout decisions worth knowing when writing raw content:
//!
//! - Every page's `/Resources` is one indirect dictionary that the page's Form
//!   XObjects and annotation appearance streams share. Content anywhere on the
//!   page can therefore name any font, form (including a form naming itself,
//!   for recursion fixtures) or optional-content group of that page.
//! - `/Helv` (Helvetica, WinAnsiEncoding, not embedded) is always present in
//!   those resources, so raw content may use `/Helv n Tf` freely. A fixture
//!   that needs a missing font resource names some other font.
//! - Coordinates passed to [`PageBuilder::insert_text`] are top-left based with
//!   y growing downwards, measured on the unrotated MediaBox; raw content is
//!   in PDF user space (bottom-left origin).

use std::fmt::Write as _;
use std::path::Path;

use crate::BuildError;
use crate::afm::encode_win_ansi;
use crate::fonts::{FontDict, write_font};
use crate::pdfwriter::{Objects, hex_string, name, num, num_array, stream_body, text_string};

/// Name of the always-present Helvetica resource.
pub const DEFAULT_FONT: &str = "Helv";

/// Style of one [`PageBuilder::insert_text`] run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// Font size in points (`Tf` operand). Default 11.
    pub fontsize: f32,
    /// Text rendering mode 0-7 (`Tr`). Default 0 (fill).
    pub render_mode: u8,
    /// Font resource name. Default [`DEFAULT_FONT`]; any other name must be
    /// added with [`PageBuilder::add_font`] (text is always WinAnsi-encoded).
    pub font: &'static str,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            fontsize: 11.0,
            render_mode: 0,
            font: DEFAULT_FONT,
        }
    }
}

impl TextStyle {
    /// Default style at the given size.
    pub fn size(fontsize: f32) -> Self {
        Self {
            fontsize,
            ..Self::default()
        }
    }
}

/// An annotation with an optional appearance. Built with [`Annotation::new`]
/// and the `with_*` methods; fields are public so tests can state odd
/// combinations directly.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    /// `/Subtype`, e.g. `FreeText`, `Stamp`, `Popup`, `Link`, `Widget`.
    pub subtype: String,
    /// `/Rect` in PDF user space.
    pub rect: [f32; 4],
    /// `/F` flags; see the `FLAG_*` constants. Written only when non-zero.
    pub flags: u32,
    /// `/AP /N`.
    pub appearance: Option<Appearance>,
    /// `/Contents`.
    pub contents: Option<String>,
    /// `/DA` (default appearance string, e.g. `"/Helv 11 Tf 0 g"`).
    pub default_appearance: Option<String>,
}

impl Annotation {
    /// Annotation flag bit 2: Hidden.
    pub const FLAG_HIDDEN: u32 = 2;
    /// Annotation flag bit 3: Print.
    pub const FLAG_PRINT: u32 = 4;
    /// Annotation flag bit 6: NoView.
    pub const FLAG_NO_VIEW: u32 = 32;

    /// An annotation with no flags, appearance or text.
    pub fn new(subtype: &str, rect: [f32; 4]) -> Self {
        Self {
            subtype: subtype.to_owned(),
            rect,
            flags: 0,
            appearance: None,
            contents: None,
            default_appearance: None,
        }
    }

    /// Sets `/F`.
    #[must_use]
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    /// Sets a single `/N` appearance stream whose `/BBox` is the rectangle's
    /// size and whose matrix is the identity.
    #[must_use]
    pub fn with_appearance(mut self, content: &str) -> Self {
        self.appearance = Some(Appearance::Stream(AppearanceStream::new(content)));
        self
    }

    /// Sets the appearance explicitly (state dictionaries, custom matrix).
    #[must_use]
    pub fn with_appearance_spec(mut self, appearance: Appearance) -> Self {
        self.appearance = Some(appearance);
        self
    }

    /// Sets `/Contents` and `/DA`, as a FreeText annotation without an
    /// appearance stream would carry them.
    #[must_use]
    pub fn with_contents(mut self, contents: &str, default_appearance: &str) -> Self {
        self.contents = Some(contents.to_owned());
        self.default_appearance = Some(default_appearance.to_owned());
        self
    }
}

/// The `/N` entry of an annotation's `/AP`.
#[derive(Clone, Debug, PartialEq)]
pub enum Appearance {
    /// `/N` is a stream.
    Stream(AppearanceStream),
    /// `/N` is a dictionary of named states (as widgets use); `/AS` selects
    /// `current`.
    States {
        /// `(state name, stream)` pairs.
        states: Vec<(String, AppearanceStream)>,
        /// Written as `/AS`.
        current: String,
    },
}

/// One appearance stream (a Form XObject).
#[derive(Clone, Debug, PartialEq)]
pub struct AppearanceStream {
    /// Content operators in form space.
    pub content: String,
    /// `/BBox`; `None` means `[0 0 width height]` of the annotation's rect.
    pub bbox: Option<[f32; 4]>,
    /// `/Matrix`; `None` omits it (identity).
    pub matrix: Option<[f32; 6]>,
}

impl AppearanceStream {
    /// Stream with the default bbox and identity matrix.
    pub fn new(content: &str) -> Self {
        Self {
            content: content.to_owned(),
            bbox: None,
            matrix: None,
        }
    }
}

/// Handle to an optional-content group added with [`PageBuilder::add_ocg`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcgRef {
    name: String,
}

impl OcgRef {
    /// The `/Properties` resource name, for `/OC /<name> BDC ... EMC`.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, PartialEq)]
struct FormSpec {
    name: String,
    bbox: [f32; 4],
    content: String,
    ocg: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct OcgSpec {
    name: String,
    on: bool,
}

/// One page under construction. Obtained from [`PdfBuilder::add_page`].
#[derive(Clone, Debug, PartialEq)]
pub struct PageBuilder {
    width: f32,
    height: f32,
    content: String,
    crop_box: Option<[f32; 4]>,
    rotation: Option<i32>,
    fonts: Vec<(String, FontDict)>,
    forms: Vec<FormSpec>,
    annotations: Vec<Annotation>,
    ocgs: Vec<OcgSpec>,
}

impl PageBuilder {
    fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            content: String::new(),
            crop_box: None,
            rotation: None,
            fonts: Vec::new(),
            forms: Vec::new(),
            annotations: Vec::new(),
            ocgs: Vec::new(),
        }
    }

    /// Page width (MediaBox) in points.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Page height (MediaBox) in points.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// Appends one text run. `point` is the baseline origin of the first
    /// glyph, with the origin at the MediaBox's top-left and y growing
    /// downwards: on an 842 pt page, `(100, 100)` puts the baseline at PDF
    /// y = 742. The run is wrapped in `q`/`Q`, so its render mode does not leak
    /// into later content.
    ///
    /// # Errors
    /// [`BuildError::NotWinAnsi`] for characters without a WinAnsi glyph
    /// (including control characters such as tab and newline);
    /// [`BuildError::RenderMode`] for a mode above 7;
    /// [`BuildError::NonFinite`] for a NaN or infinite coordinate or size.
    /// Nothing is appended on error.
    pub fn insert_text(
        &mut self,
        point: (f32, f32),
        text: &str,
        style: TextStyle,
    ) -> Result<(), BuildError> {
        if style.render_mode > 7 {
            return Err(BuildError::RenderMode(style.render_mode));
        }
        if !(point.0.is_finite() && point.1.is_finite() && style.fontsize.is_finite()) {
            return Err(BuildError::NonFinite);
        }
        let bytes = encode_win_ansi(text)?;
        // Finite inputs can still overflow here (height f32::MAX, y -f32::MAX).
        let y = self.height - point.1;
        if !y.is_finite() {
            return Err(BuildError::NonFinite);
        }
        let _ = writeln!(
            self.content,
            "q BT {} {} Tf {} Tr 1 0 0 1 {} {} Tm {} Tj ET Q",
            name(style.font),
            num(style.fontsize),
            style.render_mode,
            num(point.0),
            num(y),
            hex_string(&bytes),
        );
        Ok(())
    }

    /// Appends operators verbatim (PDF user space), followed by a newline.
    pub fn raw_content(&mut self, ops: &str) -> &mut Self {
        self.content.push_str(ops);
        self.content.push('\n');
        self
    }

    /// Sets `/CropBox`.
    pub fn set_crop_box(&mut self, rect: [f32; 4]) -> &mut Self {
        self.crop_box = Some(rect);
        self
    }

    /// Sets `/Rotate` (written as given; readers expect a multiple of 90).
    pub fn set_rotation(&mut self, degrees: i32) -> &mut Self {
        self.rotation = Some(degrees);
        self
    }

    /// Adds a font resource. Adding [`DEFAULT_FONT`] replaces the default
    /// Helvetica on this page.
    pub fn add_font(&mut self, name: &str, dict: FontDict) -> &mut Self {
        self.fonts.retain(|(n, _)| n != name);
        self.fonts.push((name.to_owned(), dict));
        self
    }

    /// Adds a Form XObject resource drawn with `/<name> Do`. Its `/Resources`
    /// are the page's, so it may use the page's fonts and forms (itself
    /// included).
    pub fn add_form_xobject(&mut self, name: &str, bbox: [f32; 4], content: &str) -> &mut Self {
        self.forms.retain(|f| f.name != name);
        self.forms.push(FormSpec {
            name: name.to_owned(),
            bbox,
            content: content.to_owned(),
            ocg: None,
        });
        self
    }

    /// Makes an already-added form optional content (`/OC` on the XObject).
    /// Unknown form names are ignored.
    pub fn set_form_ocg(&mut self, form: &str, ocg: &OcgRef) -> &mut Self {
        if let Some(f) = self.forms.iter_mut().find(|f| f.name == form) {
            f.ocg = Some(ocg.name.clone());
        }
        self
    }

    /// Adds an annotation to `/Annots`, in call order.
    pub fn add_annotation(&mut self, annot: Annotation) -> &mut Self {
        self.annotations.push(annot);
        self
    }

    /// Adds an optional-content group, initially ON or OFF in the document's
    /// default configuration, and registers it in this page's `/Properties`
    /// under `name`. Use it as `/OC /<name> BDC ... EMC` in content.
    pub fn add_ocg(&mut self, name: &str, on: bool) -> OcgRef {
        self.ocgs.retain(|o| o.name != name);
        self.ocgs.push(OcgSpec {
            name: name.to_owned(),
            on,
        });
        OcgRef {
            name: name.to_owned(),
        }
    }

    /// The content stream accumulated so far.
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// A document under construction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfBuilder {
    pages: Vec<PageBuilder>,
}

/// Object numbers of document-level OCGs: `(id, on)`.
type OcgIds = Vec<(u32, bool)>;

impl PdfBuilder {
    /// An empty document (zero pages, which is itself a valid fixture).
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a page of the given MediaBox size.
    pub fn add_page(&mut self, width: f32, height: f32) -> &mut PageBuilder {
        self.pages.push(PageBuilder::new(width, height));
        self.pages.last_mut().expect("just pushed")
    }

    /// Appends an A4 page (595 x 842 pt).
    pub fn add_page_a4(&mut self) -> &mut PageBuilder {
        self.add_page(595.0, 842.0)
    }

    /// Number of pages added.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The page added `index`-th (0-based), for callers that cannot hold the
    /// `&mut PageBuilder` that [`Self::add_page`] returns. The Python test
    /// kit is one: its page objects outlive any Rust borrow, so each one
    /// keeps its index and comes back through here.
    pub fn page_mut(&mut self, index: usize) -> Option<&mut PageBuilder> {
        self.pages.get_mut(index)
    }

    /// Serialises the document. Deterministic: same calls, same bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut objs = Objects::new();
        let catalog = objs.alloc();
        let root = objs.alloc();
        let mut ocgs: OcgIds = Vec::new();
        let mut kids = Vec::with_capacity(self.pages.len());
        for page in &self.pages {
            kids.push(write_page(&mut objs, page, root, &mut ocgs));
        }
        let kid_refs: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
        objs.set(
            root,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kid_refs.join(" "),
                kids.len()
            ),
        );
        let mut cat = format!("<< /Type /Catalog /Pages {root} 0 R");
        if !ocgs.is_empty() {
            let refs = |pred: &dyn Fn(bool) -> bool| -> String {
                let v: Vec<String> = ocgs
                    .iter()
                    .filter(|(_, on)| pred(*on))
                    .map(|(id, _)| format!("{id} 0 R"))
                    .collect();
                v.join(" ")
            };
            let all = refs(&|_| true);
            let _ = write!(
                cat,
                " /OCProperties << /OCGs [{all}] /D << /Order [{all}] /ON [{}] /OFF [{}] >> >>",
                refs(&|on| on),
                refs(&|on| !on),
            );
        }
        cat.push_str(" >>");
        objs.set(catalog, cat);
        objs.to_bytes(catalog, "")
    }

    /// Writes [`Self::to_bytes`] to `path`.
    ///
    /// # Errors
    /// Any I/O error from writing the file.
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::write(path, self.to_bytes())
    }
}

/// Writes one page and everything it owns; returns the page object number.
fn write_page(objs: &mut Objects, page: &PageBuilder, parent: u32, ocgs: &mut OcgIds) -> u32 {
    let page_id = objs.alloc();
    let resources = objs.alloc();

    let mut fonts = String::new();
    let has_default = page.fonts.iter().any(|(n, _)| n == DEFAULT_FONT);
    if !has_default {
        let id = write_font(objs, &FontDict::helvetica());
        let _ = write!(fonts, " {} {id} 0 R", name(DEFAULT_FONT));
    }
    for (n, dict) in &page.fonts {
        let id = write_font(objs, dict);
        let _ = write!(fonts, " {} {id} 0 R", name(n));
    }

    let mut props = String::new();
    let mut ocg_ids: Vec<(&str, u32)> = Vec::new();
    for o in &page.ocgs {
        let id = objs.add(format!("<< /Type /OCG /Name {} >>", text_string(&o.name)));
        ocgs.push((id, o.on));
        ocg_ids.push((&o.name, id));
        let _ = write!(props, " {} {id} 0 R", name(&o.name));
    }

    let mut xobjects = String::new();
    for f in &page.forms {
        let mut entries = format!(
            "/Type /XObject /Subtype /Form /BBox {} /Resources {resources} 0 R",
            num_array(&f.bbox)
        );
        if let Some(oc) = &f.ocg
            && let Some((_, id)) = ocg_ids.iter().find(|(n, _)| n == oc)
        {
            let _ = write!(entries, " /OC {id} 0 R");
        }
        let id = objs.add(stream_body(&entries, f.content.as_bytes()));
        let _ = write!(xobjects, " {} {id} 0 R", name(&f.name));
    }

    let mut res = format!("<< /Font <<{fonts} >>");
    if !xobjects.is_empty() {
        let _ = write!(res, " /XObject <<{xobjects} >>");
    }
    if !props.is_empty() {
        let _ = write!(res, " /Properties <<{props} >>");
    }
    res.push_str(" >>");
    objs.set(resources, res);

    let contents = objs.add(stream_body("", page.content.as_bytes()));

    let mut annots = Vec::new();
    for a in &page.annotations {
        annots.push(write_annotation(objs, a, page_id, resources));
    }

    let mut d = format!(
        "<< /Type /Page /Parent {parent} 0 R /MediaBox {}",
        num_array(&[0.0, 0.0, page.width, page.height])
    );
    if let Some(c) = page.crop_box {
        let _ = write!(d, " /CropBox {}", num_array(&c));
    }
    if let Some(r) = page.rotation {
        let _ = write!(d, " /Rotate {r}");
    }
    let _ = write!(d, " /Resources {resources} 0 R /Contents {contents} 0 R");
    if !annots.is_empty() {
        let refs: Vec<String> = annots.iter().map(|a| format!("{a} 0 R")).collect();
        let _ = write!(d, " /Annots [{}]", refs.join(" "));
    }
    d.push_str(" >>");
    objs.set(page_id, d);
    page_id
}

fn write_appearance_stream(
    objs: &mut Objects,
    s: &AppearanceStream,
    rect: [f32; 4],
    resources: u32,
) -> u32 {
    let bbox = s
        .bbox
        .unwrap_or([0.0, 0.0, rect[2] - rect[0], rect[3] - rect[1]]);
    let mut entries = format!(
        "/Type /XObject /Subtype /Form /BBox {} /Resources {resources} 0 R",
        num_array(&bbox)
    );
    if let Some(m) = s.matrix {
        let _ = write!(entries, " /Matrix {}", num_array(&m));
    }
    objs.add(stream_body(&entries, s.content.as_bytes()))
}

fn write_annotation(objs: &mut Objects, a: &Annotation, page: u32, resources: u32) -> u32 {
    let mut d = format!(
        "<< /Type /Annot /Subtype {} /Rect {} /P {page} 0 R",
        name(&a.subtype),
        num_array(&a.rect)
    );
    if a.flags != 0 {
        let _ = write!(d, " /F {}", a.flags);
    }
    if let Some(c) = &a.contents {
        let _ = write!(d, " /Contents {}", text_string(c));
    }
    if let Some(da) = &a.default_appearance {
        let _ = write!(d, " /DA {}", text_string(da));
    }
    match &a.appearance {
        None => {}
        Some(Appearance::Stream(s)) => {
            let id = write_appearance_stream(objs, s, a.rect, resources);
            let _ = write!(d, " /AP << /N {id} 0 R >>");
        }
        Some(Appearance::States { states, current }) => {
            let mut n = String::new();
            for (state, s) in states {
                let id = write_appearance_stream(objs, s, a.rect, resources);
                let _ = write!(n, " {} {id} 0 R", name(state));
            }
            let _ = write!(d, " /AP << /N <<{n} >> >> /AS {}", name(current));
        }
    }
    d.push_str(" >>");
    objs.add(d)
}
