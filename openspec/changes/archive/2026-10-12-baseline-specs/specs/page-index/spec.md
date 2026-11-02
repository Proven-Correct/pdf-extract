# page-index Specification

## Purpose

Give every opened document an honest, bounded page count from a lazy walk of its page tree, survive page-tree defects without crashing, and expose pages through ordered iteration, random access and lightweight page handles.

## ADDED Requirements

### Requirement: Page index built at open from the page tree
The system SHALL build the page index during open by walking the page tree from the catalog's `/Pages` entry, and SHALL determine the page count without interpreting or decoding any page's content streams and without reading resource values.

#### Scenario: Open does not touch page content
- **WHEN** a document with pages is opened with `open` or `open_bytes`
- **THEN** `page_count` is known when open returns, and no page content has been interpreted to produce it [F5, D7, §5.4; 0001 T9, 0001 T17; ADR-0022]

#### Scenario: Each page-tree node is read at most once
- **WHEN** a document's page tree is walked at open
- **THEN** the walk reads at most one dictionary per leaf and per interior node [§5.4; 0001 T18; ADR-0013]

### Requirement: Declared /Count is ignored
The system SHALL ignore the page tree's `/Count` entries entirely; `/Count` SHALL NOT change the page count or size any allocation.

#### Scenario: Lying /Count
- **WHEN** a page tree with 100 leaves declares `/Count 5`
- **THEN** `page_count` is 100 [D7, §5.4; 0001 T9; ADR-0026]

### Requirement: Page count equals what iteration yields
The system SHALL report a `page_count` equal to the number of pages that iteration yields, and `page_count` SHALL be answered in constant time once the document is open.

#### Scenario: Count matches iteration
- **WHEN** a 40-page document is opened and iterated
- **THEN** iteration yields exactly `page_count` pages and `len(doc) == page_count` [F5, D7; 0001 T16]

### Requirement: Leaf and interior classification by /Kids
The system SHALL treat a page-tree node that has `/Kids` as interior and a node without `/Kids` as a page, regardless of its `/Type`; the root SHALL always be read as interior.

#### Scenario: Typeless leaf
- **WHEN** a leaf dictionary has no `/Type /Page`
- **THEN** it is counted as a page and no warning is issued [§5.4; 0001 T12]

#### Scenario: Kid referring to the catalog
- **WHEN** a `/Kids` entry references the catalog, which has no `/Kids`
- **THEN** it is indexed as a page that yields empty text, and nothing loops [0001 T12]

#### Scenario: Root without /Kids
- **WHEN** the `/Pages` root has no readable `/Kids` array
- **THEN** the root yields no pages and a `PageTreeNodeUnreadable` warning is recorded [§5.4; 0001 T5, 0001 T15; ADR-0025]

### Requirement: Direct dictionary kids are pages
The system SHALL index a `/Kids` entry that is a direct dictionary (not a reference) as a page, up to position 4096 within its parent's `/Kids`.

#### Scenario: Direct dictionary kid
- **WHEN** a `/Kids` array contains a direct page dictionary
- **THEN** it is counted as a page and opens without warnings [0001 T10, 0001 T13]

#### Scenario: Direct kid beyond the position limit
- **WHEN** a direct page dictionary sits beyond position 4096 of its parent's `/Kids`
- **THEN** it is not indexed and a warning `page tree: direct kid beyond position limit` is recorded [0001 T13]

#### Scenario: Non-dictionary direct kid
- **WHEN** a `/Kids` entry is neither a reference nor a dictionary
- **THEN** it is skipped with a `PageTreeNodeUnreadable` warning [0001 T10, 0001 T13]

### Requirement: Inherited page attributes
The system SHALL give each page the `MediaBox`, `CropBox`, `Rotate` and `Resources` it inherits from its ancestors in the page tree when the page's own dictionary does not set them.

#### Scenario: Attributes inherited from an interior node
- **WHEN** a page's `MediaBox` and `Resources` are set only on its parent `/Pages` node
- **THEN** operations on the page use the inherited `MediaBox` and `Resources` [§5.4; 0001 T13]

### Requirement: Revisited nodes are skipped
The system SHALL skip any page-tree node reached a second time, whether through a cycle or a duplicate `/Kids` entry, record a `PageTreeRevisited` warning, and never raise because of it.

#### Scenario: Duplicate leaf
- **WHEN** a `/Kids` array references the same page twice
- **THEN** the page is counted once and a warning beginning `page tree: node revisited` is recorded [§5.4, KP10; 0001 T10; ADR-0023]

#### Scenario: Cycle
- **WHEN** a page-tree node lists an ancestor among its `/Kids`
- **THEN** open terminates, each page is counted once, and a `PageTreeRevisited` warning is recorded [§5.4, KP10; 0001 T10; ADR-0023]

### Requirement: Deliberate divergence on duplicates and cycles
The system SHALL keep the honest count for duplicate leaves and cycles as a documented divergence from the PyMuPDF oracle (compat point KP10).

#### Scenario: Oracle comparison
- **WHEN** a document with a duplicated leaf is opened by both systems
- **THEN** the oracle counts the leaf twice while this system yields it once, and iteration never raises on a cycle [KP10; 0001 T10, 0001 §2.5; ADR-0023]

### Requirement: Page-tree depth limit
The system SHALL skip page-tree nodes deeper than `max_page_tree_depth` (default 256, root at depth 0, overridable through `Limits`) and record a `PageTreeDepth` warning; exceeding it SHALL NOT be an error.

#### Scenario: Tree within the default depth
- **WHEN** a page tree has its single leaf at depth 200
- **THEN** the document opens with `page_count == 1` and no warning [§5.4; 0001 T11; ADR-0024]

#### Scenario: Limit is inclusive and configurable
- **WHEN** `max_page_tree_depth` is 10 and the leaf is at depth 10, or at depth 11
- **THEN** the page is counted at depth 10 and skipped at depth 11 [0001 T11; ADR-0024]

#### Scenario: Over-deep tree on a small stack
- **WHEN** a page tree 3,000 levels deep is opened on a 512 KiB thread stack
- **THEN** the document opens with `page_count == 0` and a `PageTreeDepth` warning [§5.4; 0001 T11, 0001 T15; ADR-0024]

### Requirement: Deliberate divergence on deep trees
The system SHALL treat pages deeper than `max_page_tree_depth` as a documented divergence from the oracle (compat point KP23), which has no depth limit.

#### Scenario: Oracle finds the deep page
- **WHEN** a 20,000-deep page-tree chain is opened by both systems
- **THEN** the oracle extracts the page while this system skips it with a `PageTreeDepth` warning [KP23; 0001 T11, 0001 §2.5]

### Requirement: Unreadable nodes are skipped
The system SHALL skip a page-tree node whose dictionary cannot be resolved or parsed, record a `PageTreeNodeUnreadable` warning, and continue the walk.

#### Scenario: Dangling /Pages reference
- **WHEN** the catalog's `/Pages` reference does not resolve and no `/Type /Page` dictionary exists in the file
- **THEN** the document opens with `page_count == 0` and warnings `PageTreeNodeUnreadable` and `PageTreeBruteForce` [0001 T5; ADR-0025]

#### Scenario: Over-nested page dictionary
- **WHEN** a leaf's dictionary nests deeper than the object-nesting cap
- **THEN** the document opens with that leaf skipped and a `PageTreeNodeUnreadable` warning [0001 T5, 0001 §1.6; ADR-0025]

### Requirement: Brute-force fallback scan
The system SHALL, when the walk yields no pages and no subtree was cut by the depth limit, scan every object for dictionaries with `/Type /Page`, index them in object-number order, and record a `PageTreeBruteForce` warning.

#### Scenario: Broken tree rescued by the scan
- **WHEN** the `/Pages` root has no `/Kids` but the file contains a `/Type /Page` dictionary
- **THEN** the document opens with that page, which extracts normally, and warnings `PageTreeNodeUnreadable` then `PageTreeBruteForce` [§5.4; 0001 T15; 0002 O36]

#### Scenario: Scan order
- **WHEN** the scan finds page dictionaries stored out of object-number order in the file
- **THEN** pages are indexed in ascending object-number order [§5.4; 0001 T15]

#### Scenario: Depth-cut tree is not rescued
- **WHEN** the walk yields no pages only because subtrees were cut by `max_page_tree_depth`
- **THEN** no scan runs and no `PageTreeBruteForce` warning is recorded [0001 T15; ADR-0024]

### Requirement: Empty page tree
The system SHALL open a document whose page tree and fallback scan both find no pages as a document with `page_count == 0`.

#### Scenario: Empty /Kids
- **WHEN** the `/Pages` root has an empty `/Kids` array and no `/Type /Page` dictionary exists
- **THEN** the document opens with `page_count == 0` and iteration yields nothing [0001 T5]

### Requirement: Catalog without /Pages is corrupt
The system SHALL reject a document whose catalog has no `/Pages` entry, or whose `/Pages` value is neither a reference nor a dictionary, as corrupt, as a documented divergence from the oracle.

#### Scenario: Missing /Pages key
- **WHEN** the catalog has no `/Pages` key
- **THEN** open fails with `OpenError::Corrupt` (Python `CorruptDocumentError`), while the oracle opens it with 0 pages [0001 T5, 0002 H11; ADR-0025]

### Requirement: Direct /Pages dictionary
The system SHALL accept a catalog whose `/Pages` value is a direct dictionary and walk it as the root of the page tree, without a warning.

#### Scenario: Inline root
- **WHEN** a catalog's `/Pages` value is an inline dictionary with one page
- **THEN** the document opens with `page_count == 1`, the page's text is `Hello\n`, and no warning is recorded [0002 O36; ADR-0075]

#### Scenario: Inline root and max_pages
- **WHEN** that document is opened with `max_pages = 0`
- **THEN** open fails with `TooManyPagesError` [0002 O36]

### Requirement: max_pages bounds open
The system SHALL, when opened with `max_pages = N`, fail with `TooManyPagesError` (Rust `OpenError::TooManyPages { limit: N, at_least: N + 1 }`) as soon as page N+1 is found, so open costs O(N + depth) node reads however many pages the file holds.

#### Scenario: Limit exceeded
- **WHEN** a 10,000-page document is opened with `max_pages=10`
- **THEN** open raises `TooManyPagesError` with `limit == 10` and `at_least == 11`, a subclass of `OpenError` [F5, D7; 0001 T14, 0001 T47b; ADR-0022]

#### Scenario: Exactly at the limit
- **WHEN** a 10-page document is opened with `max_pages=10`
- **THEN** it opens with `page_count == 10` [F5; 0001 T14]

#### Scenario: Limit applies to the fallback scan
- **WHEN** the brute-force scan finds more than `max_pages` page dictionaries
- **THEN** open fails with `TooManyPagesError` [§5.4; 0001 T15]

#### Scenario: No limit by default
- **WHEN** `max_pages` is not given
- **THEN** the 10,000-page document opens with `page_count == 10000` [F5; 0001 §3.5]

#### Scenario: Negative max_pages in Python
- **WHEN** `open` or `open_bytes` is called with `max_pages=-1`
- **THEN** it raises `ValueError` naming `max_pages` [0001 §3.5]

### Requirement: Ordered iteration
The system SHALL iterate a document's pages in document order, yielding each page exactly once, through an exact-size iterator (Rust `Document::pages()`, Python `iter(doc)`).

#### Scenario: Iteration order
- **WHEN** a 40-page document is iterated
- **THEN** the yielded pages have indices 0 through 39 in order [F6; 0001 T16]

#### Scenario: Remaining length
- **WHEN** one page has been taken from a fresh iterator over 40 pages
- **THEN** the iterator reports 39 remaining (Rust `len()`, Python `__length_hint__`), and after exhaustion it keeps returning nothing [F6; 0001 T16, 0001 §3.5]

#### Scenario: Shared iterator
- **WHEN** one iterator is consumed concurrently by several threads
- **THEN** every page index is handed out exactly once across the threads [F6; 0001 T16, 0001 T43]

### Requirement: Random access by index
The system SHALL give random access to a page by index: Rust `Document::page(i)` returns `PageError::IndexOutOfRange { index, count }` out of range, and Python `doc[i]` accepts negative indices counted from the end and raises `IndexError` out of range.

#### Scenario: Negative index
- **WHEN** `doc[-1]` and `doc[-40]` are taken on a 40-page document
- **THEN** they return the pages with index 39 and 0 [F6; 0001 T46]

#### Scenario: Out of range
- **WHEN** `doc[40]`, `doc[-41]`, `doc[2**70]` or `doc[-(2**70)]` is taken on a 40-page document
- **THEN** each raises `IndexError` [F6; 0001 T46, 0001 T47b]

#### Scenario: Non-integer index
- **WHEN** `doc[i]` is called with a string, float, `None`, slice or list
- **THEN** it raises `TypeError` with a message beginning `page indices must be integers` [0001 T46]

#### Scenario: Integer-like index
- **WHEN** `doc[i]` is called with an object implementing `__index__`, or with `True`
- **THEN** it is accepted as that integer, and an exception raised by `__index__` propagates unchanged [0001 T46]

### Requirement: Lightweight page handles
The system SHALL represent a page as a lightweight handle holding only its index and a reference to its document, exposing that index (Rust `Page::index()`, Python `Page.index`), and SHALL NOT cache parsed page state in the handle or the document.

#### Scenario: Handle index
- **WHEN** `doc.page(1)` (Rust) or `doc[1]` (Python) is taken
- **THEN** the handle's index is 1 [F6; 0001 T16, 0001 T17]

### Requirement: Page index facts after close
The system SHALL keep `page_count` and `len(doc)` answerable after `close()`, SHALL keep yielding page handles from iteration after `close()`, and SHALL fail operations through those handles and new random access with `PageError::Closed` (Python `DocumentClosedError`).

#### Scenario: Use after close
- **WHEN** a one-page document is closed
- **THEN** `page_count` and `len(doc)` still return 1, iteration still yields a handle with index 0, and `doc[0]` and that handle's `text()` raise `DocumentClosedError` [F6; 0001 T16, 0001 T40, 0001 T47b]

### Requirement: Open-time performance target
The system SHALL target open plus `page_count` within 100 ms for a 100,000-page file and within 1.5 s and 150 MB above the file size for a 1,000,000-page file, measured by a report-only benchmark that never gates CI.

#### Scenario: P1 benchmark
- **WHEN** the CI benchmark opens generated 100k- and 1M-page files, and the 1M file with `max_pages=10`
- **THEN** timings and peak RSS are reported against the recorded baseline without failing the pipeline [P1; 0001 T18; ADR-0013]
