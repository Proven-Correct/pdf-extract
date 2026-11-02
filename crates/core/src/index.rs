//! The page index (spec T9-T15, T17; design spec §5.4).
//!
//! Opening a document walks `catalog./Pages` once and records, per page, just
//! enough to rebuild that page later: a 16-byte [`PageRef`] per leaf plus one
//! [`InheritedNode`] per interior node. No hayro `Page` is built at open and
//! no `/Contents` or `/Resources` value is read; a page is assembled with
//! hayro's `Page::from_dict` (fork change H1) only inside an operation, on
//! that operation's stack, and dropped with it (T17).
//!
//! The walk is iterative with an explicit stack of `/Kids` iterators, so its
//! stack use is constant and its heap use is O(depth); depth is capped by
//! `Limits::max_page_tree_depth`. Every node is resolved at most once
//! (deduplicated by object identifier), so the walk resolves at most
//! `leaves + interior nodes` dictionaries, and with `max_pages = n` it stops
//! after O(n + depth) resolutions (T14).
//!
//! Classification follows T12, not `/Type`: a node with `/Kids` is interior,
//! anything else is a leaf. The root is the exception: it is always read as
//! interior, so a root without `/Kids` yields no pages (D-31) rather than
//! being taken for a page.
//!
//! The root may be written inline in the catalog (fork change H11, run 0002
//! O36). It then has no identifier: it is not deduplicated (nothing can
//! refer back to it), its [`InheritedNode`] is flagged [`INLINE_ROOT`], and
//! whenever a page needs it again it is re-read from the catalog. Its direct
//! kids are found again by position, like any other direct kid.

use std::collections::HashSet;

use hayro_syntax::object::dict::keys::{CROP_BOX, KIDS, MEDIA_BOX, PAGES, RESOURCES, ROTATE, TYPE};
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, ObjectIdentifier, Rect};
use hayro_syntax::page::{InheritedAttributes, Page as HayroPage};
use hayro_syntax::xref::XRef;
use hayro_syntax::{PagesRoot, Pdf, object::array::ArrayIter};

use crate::error::OpenError;
use crate::warning::{WarningKind, WarningSink};

/// Sentinel for "no parent node" ([`PageRef::inherit`] of brute-force
/// leaves, [`InheritedNode::parent`] of the root).
const NONE: u32 = u32::MAX;

/// [`PageRef::kid`] of an indirect leaf.
const NOT_DIRECT: u16 = u16::MAX;

/// Stand-in identifier of an inline page-tree root (H11), which has none of
/// its own. It is only ever recorded, never resolved: the node carries
/// [`INLINE_ROOT`] and is re-read from the catalog instead, so a real object
/// `0 0 R` elsewhere in the file cannot be mistaken for it.
const INLINE_ROOT_ID: ObjectIdentifier = ObjectIdentifier {
    obj_number: 0,
    gen_number: 0,
};

/// Highest `/Kids` position at which a direct-dictionary leaf is accepted.
/// A direct leaf has no identifier, so rebuilding its page re-reads the
/// parent's `/Kids` up to its position: unbounded positions would make
/// rebuilding every page quadratic in the array length (a few megabytes of
/// direct kids cost minutes). Real files put at most a handful of direct
/// kids in a node; beyond this position they are skipped with a warning.
pub(crate) const MAX_DIRECT_KID_POSITION: usize = 4096;

/// Where one page's dictionary lives and which interior node it inherits
/// from. Exactly 16 bytes (asserted below), so a million-page index costs
/// 16 MB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PageRef {
    /// The leaf's own object identifier; for a leaf written as a direct
    /// dictionary inside its parent's `/Kids`, the parent's identifier
    /// (informational: such a leaf is rebuilt through its parent's
    /// [`InheritedNode`], which knows whether that parent is an inline root).
    obj: ObjectIdentifier,
    /// Index into [`PageIndex::inherited`] of the leaf's parent, or `NONE`.
    inherit: u32,
    /// `NOT_DIRECT` for an indirect leaf; otherwise the leaf's position in the
    /// parent's `/Kids` (direct dictionaries have no identifier of their own,
    /// so they are found again by position).
    kid: u16,
}

const _: () = assert!(size_of::<PageRef>() == 16);
const _: () = assert!(MAX_DIRECT_KID_POSITION < NOT_DIRECT as usize);

/// An interior node: its identifier, its parent, and which inheritable keys
/// it carries, so that rebuilding a page resolves only the ancestors that
/// actually contribute something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InheritedNode {
    id: ObjectIdentifier,
    parent: u32,
    attrs: u8,
}

const HAS_MEDIA_BOX: u8 = 1;
const HAS_CROP_BOX: u8 = 2;
const HAS_ROTATE: u8 = 4;
const HAS_RESOURCES: u8 = 8;
/// The node is the catalog's inline `/Pages` dictionary (H11); its `id` is
/// [`INLINE_ROOT_ID`] and must not be resolved.
const INLINE_ROOT: u8 = 16;

/// The page index of an opened document.
#[derive(Debug, Default)]
pub(crate) struct PageIndex {
    leaves: Vec<PageRef>,
    inherited: Vec<InheritedNode>,
}

/// Something the walk can resolve page-tree dictionaries through. The real
/// one is hayro's `XRef`; tests wrap it to count resolutions (T18).
pub(crate) trait NodeSource<'a> {
    /// The dictionary with identifier `id`, or `None` when it is missing,
    /// unparseable or not a dictionary.
    fn dict(&self, id: ObjectIdentifier) -> Option<Dict<'a>>;
}

impl<'a> NodeSource<'a> for &'a XRef {
    fn dict(&self, id: ObjectIdentifier) -> Option<Dict<'a>> {
        self.get::<Dict<'a>>(id)
    }
}

/// Bounds for one walk.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WalkLimits {
    pub(crate) max_pages: Option<usize>,
    pub(crate) max_depth: u32,
}

/// One open `/Kids` array on the walk's stack.
struct Frame<'a> {
    kids: ArrayIter<'a>,
    /// Position of the next kid in `kids`.
    position: usize,
    /// Depth of the node that owns `kids` (the root is 0).
    depth: u32,
    /// That node's index in `PageIndex::inherited` and its identifier
    /// (`None` for an inline root, which has none).
    node: u32,
    node_id: Option<ObjectIdentifier>,
}

/// State of one walk.
struct Walk<'s, 'a, S> {
    source: &'s S,
    limits: WalkLimits,
    sink: &'s mut WarningSink,
    index: PageIndex,
    visited: HashSet<ObjectIdentifier>,
    stack: Vec<Frame<'a>>,
    depth_skipped: bool,
}

impl<'a, S: NodeSource<'a>> Walk<'_, 'a, S> {
    /// Records a leaf, enforcing `max_pages` (T14): the error is raised when
    /// leaf `n + 1` is found, before it is stored.
    fn leaf(&mut self, leaf: PageRef) -> Result<(), OpenError> {
        if let Some(limit) = self.limits.max_pages
            && self.index.leaves.len() >= limit
        {
            return Err(OpenError::TooManyPages {
                limit,
                at_least: limit + 1,
            });
        }
        self.index.leaves.push(leaf);
        Ok(())
    }

    /// Visits the node `id` at `depth` whose parent is interior node
    /// `parent` (`NONE` for the root).
    fn visit(&mut self, id: ObjectIdentifier, depth: u32, parent: u32) -> Result<(), OpenError> {
        // Depth first, so a too-deep subtree costs no resolution at all.
        if depth > self.limits.max_depth {
            self.depth_skipped = true;
            let max = self.limits.max_depth;
            self.sink.push(WarningKind::PageTreeDepth, || {
                format!("node {} deeper than {max} skipped", fmt_id(id))
            });
            return Ok(());
        }
        if !self.visited.insert(id) {
            self.sink.push(WarningKind::PageTreeRevisited, || {
                format!("node revisited ({})", fmt_id(id))
            });
            return Ok(());
        }
        let Some(dict) = self.source.dict(id) else {
            self.sink.push(WarningKind::PageTreeNodeUnreadable, || {
                format!("node unreadable ({})", fmt_id(id))
            });
            return Ok(());
        };
        let is_root = parent == NONE;
        if !is_root && !dict.contains_key(KIDS) {
            return self.leaf(PageRef {
                obj: id,
                inherit: parent,
                kid: NOT_DIRECT,
            });
        }
        self.interior(&dict, Some(id), depth, parent)
    }

    /// Records `dict` as an interior node (identifier `id`, `None` for an
    /// inline root) and queues its `/Kids`.
    fn interior(
        &mut self,
        dict: &Dict<'a>,
        id: Option<ObjectIdentifier>,
        depth: u32,
        parent: u32,
    ) -> Result<(), OpenError> {
        let Some(kids) = dict.get::<Array<'a>>(KIDS) else {
            self.sink.push(WarningKind::PageTreeNodeUnreadable, || {
                format!("node {} has no readable /Kids array", fmt_node(id))
            });
            return Ok(());
        };
        let node = u32::try_from(self.index.inherited.len())
            .ok()
            .filter(|&n| n != NONE)
            .ok_or_else(|| OpenError::Corrupt("page tree has too many nodes".into()))?;
        let inline = if id.is_none() { INLINE_ROOT } else { 0 };
        self.index.inherited.push(InheritedNode {
            id: id.unwrap_or(INLINE_ROOT_ID),
            parent,
            attrs: inheritable_attrs(dict) | inline,
        });
        self.stack.push(Frame {
            kids: kids.raw_iter(),
            position: 0,
            depth,
            node,
            node_id: id,
        });
        Ok(())
    }

    /// Handles a `/Kids` entry that is not a reference. T10: a direct
    /// dictionary is a leaf, whatever it holds, and has no identifier to
    /// deduplicate it by. It is still subject to the depth cap (it sits one
    /// level below its parent) and to [`MAX_DIRECT_KID_POSITION`].
    fn direct_kid(
        &mut self,
        is_dict: bool,
        position: usize,
        parent_depth: u32,
        node: u32,
        node_id: Option<ObjectIdentifier>,
    ) -> Result<(), OpenError> {
        if !is_dict {
            self.sink.push(WarningKind::PageTreeNodeUnreadable, || {
                format!(
                    "entry {position} of /Kids in {} is not a page reference",
                    fmt_node(node_id)
                )
            });
            return Ok(());
        }
        if parent_depth >= self.limits.max_depth {
            self.depth_skipped = true;
            let max = self.limits.max_depth;
            self.sink.push(WarningKind::PageTreeDepth, || {
                format!(
                    "direct page dictionary in {} deeper than {max} skipped",
                    fmt_node(node_id)
                )
            });
            return Ok(());
        }
        // The cap is far below `NOT_DIRECT`, so an accepted position always
        // converts and never collides with the sentinel.
        match u16::try_from(position) {
            Ok(kid) if position <= MAX_DIRECT_KID_POSITION => self.leaf(PageRef {
                obj: node_id.unwrap_or(INLINE_ROOT_ID),
                inherit: node,
                kid,
            }),
            _ => {
                self.sink.push(WarningKind::PageTreeNodeUnreadable, || {
                    format!(
                        "direct kid beyond position limit ({MAX_DIRECT_KID_POSITION}): \
                         /Kids position {position} in {} skipped",
                        fmt_node(node_id)
                    )
                });
                Ok(())
            }
        }
    }

    /// Runs the walk from `root` to completion.
    fn run(&mut self, root: PagesRoot<'a>) -> Result<(), OpenError> {
        match root {
            PagesRoot::Ref(id) => self.visit(id, 0, NONE)?,
            // Depth 0 is never over the cap, and an inline root has no
            // identifier to deduplicate or resolve: it costs no resolution.
            PagesRoot::Inline(dict) => self.interior(&dict, None, 0, NONE)?,
        }
        while let Some(frame) = self.stack.last_mut() {
            let Some(kid) = frame.kids.next() else {
                self.stack.pop();
                continue;
            };
            let position = frame.position;
            frame.position += 1;
            let (depth, node, node_id) = (frame.depth, frame.node, frame.node_id);
            match kid {
                MaybeRef::Ref(r) => self.visit(r.into(), depth + 1, node)?,
                MaybeRef::NotRef(obj) => {
                    self.direct_kid(obj.into_dict().is_some(), position, depth, node, node_id)?
                }
            }
        }
        Ok(())
    }
}

/// Which inheritable keys `dict` carries (presence only; values are read
/// when a page is built).
fn inheritable_attrs(dict: &Dict<'_>) -> u8 {
    [
        (MEDIA_BOX, HAS_MEDIA_BOX),
        (CROP_BOX, HAS_CROP_BOX),
        (ROTATE, HAS_ROTATE),
        (RESOURCES, HAS_RESOURCES),
    ]
    .iter()
    .filter(|(key, _)| dict.contains_key(key))
    .fold(0, |acc, (_, bit)| acc | bit)
}

fn fmt_id(id: ObjectIdentifier) -> String {
    format!("{} {} R", id.obj_number, id.gen_number)
}

/// [`fmt_id`] for a node that may be the inline root.
fn fmt_node(id: Option<ObjectIdentifier>) -> String {
    id.map_or_else(|| "the inline /Pages root".to_owned(), fmt_id)
}

/// The catalog's inline `/Pages` dictionary, read again (H11).
fn inline_root<'a>(xref: &'a XRef) -> Option<Dict<'a>> {
    xref.get::<Dict<'a>>(xref.root_id())?.get::<Dict<'a>>(PAGES)
}

/// What [`walk`] found.
#[derive(Debug)]
pub(crate) struct WalkOutcome {
    pub(crate) index: PageIndex,
    /// Whether any subtree was cut by `max_depth`. A walk that found no
    /// pages *because* of the depth cap is not retried by brute force: that
    /// would resurrect exactly the pages the cap is there to refuse (spec
    /// §4.8: `deep_page_tree(3000)` opens with 0 pages).
    pub(crate) depth_skipped: bool,
}

/// Walks the page tree rooted at `root` through `source`.
pub(crate) fn walk<'a>(
    source: &impl NodeSource<'a>,
    root: PagesRoot<'a>,
    limits: WalkLimits,
    sink: &mut WarningSink,
) -> Result<WalkOutcome, OpenError> {
    let mut w = Walk {
        source,
        limits,
        sink,
        index: PageIndex::default(),
        visited: HashSet::new(),
        stack: Vec::new(),
        depth_skipped: false,
    };
    w.run(root)?;
    let mut index = w.index;
    index.leaves.shrink_to_fit();
    index.inherited.shrink_to_fit();
    Ok(WalkOutcome {
        index,
        depth_skipped: w.depth_skipped,
    })
}

/// The fallback of T15: every dictionary in the file with `/Type /Page`, in
/// object-number order, bounded by `max_pages`. Used when the walk found no
/// page at all. The pages inherit nothing (their tree is unusable).
pub(crate) fn brute_force(
    pdf: &Pdf,
    max_pages: Option<usize>,
    sink: &mut WarningSink,
) -> Result<PageIndex, OpenError> {
    let mut ids = Vec::new();
    for object in pdf.objects() {
        let Some(dict) = object.into_dict() else {
            continue;
        };
        if dict.get::<Name<'_>>(TYPE).as_deref() != Some(b"Page".as_slice()) {
            continue;
        }
        let Some(id) = dict.obj_id() else {
            continue;
        };
        // `objects()` yields file order, so the limit is checked on the
        // count, which does not depend on order; sorting comes after.
        if let Some(limit) = max_pages
            && ids.len() >= limit
        {
            return Err(OpenError::TooManyPages {
                limit,
                at_least: limit + 1,
            });
        }
        ids.push(id);
    }
    ids.sort_unstable();
    ids.dedup();
    let found = ids.len();
    sink.push(WarningKind::PageTreeBruteForce, || {
        format!("brute-force scan found {found} page objects")
    });
    Ok(PageIndex {
        leaves: ids
            .into_iter()
            .map(|obj| PageRef {
                obj,
                inherit: NONE,
                kid: NOT_DIRECT,
            })
            .collect(),
        inherited: Vec::new(),
    })
}

impl PageIndex {
    /// Number of pages.
    pub(crate) fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Whether the index has no pages.
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }
}

impl PageIndex {
    /// Builds the hayro page for leaf `i` (T17): resolves the leaf's
    /// dictionary and those ancestors that carry inheritable keys, then calls
    /// `Page::from_dict`. Returns `None` when `i` is out of range or the leaf
    /// no longer resolves (possible only if a lazy xref repair after open
    /// changed what its identifier points to).
    pub(crate) fn hayro_page<'a>(&self, xref: &'a XRef, i: usize) -> Option<HayroPage<'a>> {
        let leaf = *self.leaves.get(i)?;
        let dict = if leaf.kid == NOT_DIRECT {
            xref.get::<Dict<'a>>(leaf.obj)?
        } else {
            let parent = self.node_dict(xref, leaf.inherit)?;
            let kids = parent.get::<Array<'a>>(KIDS)?;
            match kids.raw_iter().nth(leaf.kid as usize)? {
                MaybeRef::NotRef(obj) => obj.into_dict()?,
                MaybeRef::Ref(_) => return None,
            }
        };
        Some(HayroPage::from_dict(
            &dict,
            &self.inherited_attributes(xref, leaf.inherit),
        ))
    }

    /// What a leaf under interior node `node` inherits: the nearest valid
    /// `MediaBox`, `CropBox` and `Rotate`, and every ancestor's `Resources`
    /// from the root down, exactly as hayro's eager walk collects them.
    fn inherited_attributes<'a>(&self, xref: &'a XRef, mut node: u32) -> InheritedAttributes<'a> {
        let mut out = InheritedAttributes::default();
        while let Some(n) = self.inherited.get(node as usize) {
            let here = node;
            node = n.parent;
            if n.attrs & !INLINE_ROOT == 0 {
                continue;
            }
            let Some(dict) = self.node_dict(xref, here) else {
                continue;
            };
            if out.media_box.is_none() && n.attrs & HAS_MEDIA_BOX != 0 {
                out.media_box = dict.get::<Rect>(MEDIA_BOX);
            }
            if out.crop_box.is_none() && n.attrs & HAS_CROP_BOX != 0 {
                out.crop_box = dict.get::<Rect>(CROP_BOX);
            }
            if out.rotate.is_none() && n.attrs & HAS_ROTATE != 0 {
                out.rotate = dict.get::<i32>(ROTATE);
            }
            if n.attrs & HAS_RESOURCES != 0
                && let Some(res) = dict.get::<Dict<'a>>(RESOURCES)
            {
                out.resources.push(res);
            }
        }
        // Collected nearest-first; `from_dict` wants root-first.
        out.resources.reverse();
        out
    }

    /// The dictionary of interior node `node`: resolved by identifier, or
    /// re-read from the catalog for an inline root.
    fn node_dict<'a>(&self, xref: &'a XRef, node: u32) -> Option<Dict<'a>> {
        let n = self.inherited.get(node as usize)?;
        if n.attrs & INLINE_ROOT != 0 {
            inline_root(xref)
        } else {
            xref.get::<Dict<'a>>(n.id)
        }
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
