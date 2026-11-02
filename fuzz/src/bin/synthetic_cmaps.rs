//! Writes hand-written text CMaps into a directory, as seeds for the `cmap`
//! target (design D8 "Seeds"). `tools/fuzz/run.sh` and the `hardening`
//! workflow run it beside `tools/fuzz/seed-cmaps.py`, which adds the CMap
//! streams of corpus B and C.
//!
//! Usage: `synthetic_cmaps <out-dir>`.
//!
//! The corpus streams are almost all `bfchar` ToUnicode maps with a 1- or
//! 2-byte codespace, so these seeds cover the rest of the text grammar:
//! `cidrange`/`cidchar`, `notdefrange`/`notdefchar`, `bfrange` in its offset
//! and array forms, 1-, 2-, 4-byte and mixed-width codespaces, the header
//! entries, and `usecmap` naming a predefined CMap (which keeps the resolver
//! path seeded). This replaces the earlier seed set, the embedded CMaps
//! themselves: those are `bcmap` data, which the target drops since H24
//! (internal #121, ADR 0090), so they reached no parser.
//!
//! Each seed is checked before it is written: it must not start with `bcmap`,
//! must parse with the resolver the target uses, and must give the lookup
//! result its shape is there to produce. A seed that stops reaching the
//! parser (a hayro-cmap bump, a gate like H24's) fails the run instead of
//! going quietly dead again.

use std::path::Path;
use std::process::ExitCode;

use hayro_cmap::{BfString, CMap, CMapName, WritingMode, load_embedded};

/// One seed: its file name, its text, and the lookup that shows its shape
/// was parsed (not just that `parse` returned something).
struct Seed {
    name: &'static str,
    text: &'static str,
    check: fn(&CMap) -> bool,
}

fn cid(cmap: &CMap, code: u32, byte_len: u8) -> Option<u32> {
    cmap.lookup_cid_code(code, byte_len)
}

fn bf(cmap: &CMap, code: u32) -> Option<BfString> {
    cmap.lookup_bf_string(code)
}

/// The predefined CMap `name` parsed on its own, to compare an inherited
/// lookup with.
fn predefined(name: &str) -> Option<CMap> {
    CMap::parse(
        load_embedded(CMapName::from_bytes(name.as_bytes()))?,
        load_embedded,
    )
}

/// `code` resolves in `cmap` exactly as in the predefined CMap `base`, and
/// to something: the `usecmap` chain was followed.
fn inherits(cmap: &CMap, base: &str, code: u32, byte_len: u8) -> bool {
    let Some(base) = predefined(base) else {
        return false;
    };
    let got = cid(cmap, code, byte_len);
    got.is_some() && got == cid(&base, code, byte_len)
}

const SEEDS: &[Seed] = &[
    Seed {
        name: "cidrange-2byte",
        text: "%!PS-Adobe-3.0 Resource-CMap
/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Identity) def
  /Supplement 0 def
end def
/CMapName /Seed-CIDRange-H def
/CMapType 1 def
/WMode 0 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
3 begincidrange
<0000> <00FF> 0
<0100> <01FF> 1000
<FF00> <FFFF> 65280
endcidrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end
",
        check: |c| {
            cid(c, 0x0105, 2) == Some(1005)
                && cid(c, 0xFF01, 2) == Some(0xFF01)
                && c.metadata().name.as_deref() == Some(b"Seed-CIDRange-H".as_slice())
                && c.metadata().character_collection.is_some()
        },
    },
    Seed {
        name: "cidchar-2byte",
        text: "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
3 begincidchar
<0041> 34
<0042> 35
<3042> 842
endcidchar
1 begincidrange
<0061> <007A> 66
endcidrange
endcmap
end
end
",
        check: |c| cid(c, 0x0041, 2) == Some(34) && cid(c, 0x3042, 2) == Some(842),
    },
    Seed {
        name: "codespace-1byte",
        text: "begincmap
1 begincodespacerange
<00> <FF>
endcodespacerange
1 begincidrange
<20> <7E> 1
endcidrange
1 begincidchar
<A0> 1
endcidchar
endcmap
",
        check: |c| cid(c, 0x41, 1) == Some(34) && cid(c, 0xA0, 1) == Some(1),
    },
    Seed {
        // Shift-JIS shaped: single bytes for ASCII and half-width katakana,
        // two bytes behind the lead bytes 0x81-0x9F and 0xE0-0xFC.
        name: "codespace-mixed",
        text: "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 2 >> def
/CMapName /Seed-Mixed-H def
4 begincodespacerange
<00> <80>
<8140> <9FFC>
<A0> <DF>
<E040> <FCFC>
endcodespacerange
2 begincidrange
<20> <7E> 231
<A1> <DF> 326
endcidrange
2 begincidrange
<8140> <817E> 633
<E040> <E07E> 7633
endcidrange
endcmap
end
end
",
        check: |c| {
            cid(c, 0x41, 1) == Some(264)
                && cid(c, 0x8141, 2) == Some(634)
                && cid(c, 0xE040, 2) == Some(7633)
                && cid(c, 0x8141, 1).is_none()
        },
    },
    Seed {
        name: "codespace-4byte",
        text: "begincmap
1 begincodespacerange
<00000000> <FFFFFFFF>
endcodespacerange
1 begincidrange
<00010000> <0001FFFF> 100
endcidrange
1 begincidchar
<FFFFFFFF> 7
endcidchar
endcmap
",
        check: |c| cid(c, 0x0001_0005, 4) == Some(105) && cid(c, 0xFFFF_FFFF, 4) == Some(7),
    },
    Seed {
        name: "notdefrange",
        text: "begincmap
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 beginnotdefrange
<0000> <001F> 1
endnotdefrange
1 begincidrange
<0020> <00FF> 100
endcidrange
endcmap
",
        // Every code of a notdef range gives the same CID.
        check: |c| cid(c, 0x0005, 2) == Some(1) && cid(c, 0x001F, 2) == Some(1),
    },
    Seed {
        name: "notdefchar",
        text: "begincmap
1 begincodespacerange
<00> <FF>
endcodespacerange
2 beginnotdefchar
<00> 1
<7F> 2
endnotdefchar
1 begincidrange
<20> <7E> 1
endcidrange
endcmap
",
        check: |c| cid(c, 0x7F, 1) == Some(2) && cid(c, 0x41, 1) == Some(34),
    },
    Seed {
        name: "bfrange-offset",
        text: "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
2 beginbfrange
<0041> <005A> <0041>
<0100> <0103> <00E0>
endbfrange
1 beginbfchar
<0020> <0020>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end
",
        check: |c| {
            matches!(bf(c, 0x0043), Some(BfString::Char('C')))
                && matches!(bf(c, 0x0102), Some(BfString::Char('\u{E2}')))
        },
    },
    Seed {
        // The array form gives each code its own destination, here a
        // ligature (two units) and a surrogate pair among them.
        name: "bfrange-array",
        text: "begincmap
1 begincodespacerange
<00> <FF>
endcodespacerange
2 beginbfrange
<61> <63> [<0078> <0079> <007A>]
<F0> <F2> [<00660066> <D835DC00> <00660069>]
endbfrange
endcmap
",
        check: |c| {
            matches!(bf(c, 0x62), Some(BfString::Char('y')))
                && matches!(bf(c, 0xF0), Some(BfString::String(ref s)) if s == "ff")
                && matches!(bf(c, 0xF1), Some(BfString::Char('\u{1D400}')))
        },
    },
    Seed {
        // A vertical CMap: the header's WMode is the only place it shows.
        name: "header-vertical",
        text: "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Korea1) def
  /Supplement 1 def
end def
/CMapName /Seed-V def
/CMapVersion 1.0 def
/CMapType 1 def
/WMode 1 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 begincidrange
<0000> <FFFF> 0
endcidrange
endcmap
end
end
",
        check: |c| {
            c.metadata().writing_mode == Some(WritingMode::Vertical)
                && cid(c, 0x1234, 2) == Some(0x1234)
        },
    },
    Seed {
        // `usecmap` of a 2-byte predefined CMap; the local `cidchar`
        // overrides one code and the rest are inherited.
        name: "usecmap-unijis",
        text: "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/UniJIS-UCS2-H usecmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> def
/CMapName /Seed-UniJIS-Override def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 begincidchar
<0041> 9999
endcidchar
endcmap
end
end
",
        check: |c| cid(c, 0x0041, 2) == Some(9999) && inherits(c, "UniJIS-UCS2-H", 0x3042, 2),
    },
    Seed {
        // `usecmap` of a mixed-width predefined CMap, inherited at both widths.
        name: "usecmap-rksj",
        text: "begincmap
/90ms-RKSJ-H usecmap
1 begincidrange
<F040> <F07E> 8359
endcidrange
endcmap
",
        check: |c| {
            cid(c, 0xF041, 2) == Some(8360)
                && inherits(c, "90ms-RKSJ-H", 0x41, 1)
                && inherits(c, "90ms-RKSJ-H", 0x82A0, 2)
        },
    },
    Seed {
        // `usecmap` of Identity-H, the predefined CMap documents name most.
        name: "usecmap-identity",
        text: "begincmap
/Identity-H usecmap
1 beginnotdefrange
<0000> <0002> 0
endnotdefrange
endcmap
",
        check: |c| inherits(c, "Identity-H", 0x1234, 2),
    },
];

fn main() -> ExitCode {
    let Some(out) = std::env::args().nth(1) else {
        eprintln!("usage: synthetic_cmaps <out-dir>");
        return ExitCode::from(2);
    };
    let out = Path::new(&out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("cannot create {}: {e}", out.display());
        return ExitCode::from(2);
    }
    let mut bad = 0;
    for seed in SEEDS {
        let data = seed.text.as_bytes();
        // The target's own gate (`fuzz_api::parse_cmap`) drops `bcmap` data
        // before parsing, and parses with this resolver.
        let reaches_parser = !data.starts_with(b"bcmap");
        let parsed = CMap::parse(data, load_embedded);
        if !reaches_parser || !parsed.as_ref().is_some_and(seed.check) {
            eprintln!("seed {} does not parse to its intended map", seed.name);
            bad += 1;
            continue;
        }
        if let Err(e) = std::fs::write(out.join(format!("synthetic-{}", seed.name)), data) {
            eprintln!("cannot write {}: {e}", seed.name);
            return ExitCode::from(2);
        }
    }
    if bad > 0 {
        return ExitCode::FAILURE;
    }
    println!(
        "{} synthetic CMaps written to {}",
        SEEDS.len(),
        out.display()
    );
    ExitCode::SUCCESS
}
