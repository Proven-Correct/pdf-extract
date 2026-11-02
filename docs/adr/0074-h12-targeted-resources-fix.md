---
id: 0074
title: H12 is a targeted fresh-context fix, not a general parent-chain reset
date: 2026-10-03
status: Accepted
summary: H12 gives a referenced /Resources a fresh context and adds a property getter with the parent walk, instead of resetting the parent chain on every lazy lookup.
---

# 0074. H12 is a targeted fresh-context fix, not a general parent-chain reset

## Context

Spec 0002 D-19. The general change touches every lazy Dict::get and needs a full parity re-run.

## Decision

H12 is the targeted fix: fresh context for a referenced `/Resources`; property getter with the parent walk. Not a general reset of hayro-syntax's parent chain on lazy lookups.

## Alternatives considered

- **General reset of the parent chain** - touches every lazy `Dict::get`; needs a full parity re-run.

## Consequences

Closes the observed leak with three fixtures and is upstreamable. Unit 03 write-backs: lookup goes through public Resources::resolve_dict, and get_property has no production caller.

## Addendum (2026-10-21)

The precedence rule this decision relies on, stated outright (internal issue #99 D5): **a form's own `/Resources` is searched first, then its parents', so the innermost dictionary that names a resource wins.** A name that the form's `/Resources` does not carry falls through to the page's. `x_object.rs` draws a form with `Resources::from_parent(form, current)`, and every `Resources` getter, `get_property` among them, tries its own dictionary before its parent's. The `BDC` property cache is keyed on the level that held the entry (spec 0002 H12).

The corpus-C recipes in `crates/testkit/tests/common/corpus/content.rs` pin it under spec 0002 H12/O37: `ocg_forms` (the shared, inherited and direct-without-`/Properties` shapes that leaked hidden optional content before H12, plus four controls that never did) and `ocg_form_precedence` (the page's `/Properties` and the form's own name the same `P1` for different groups: `ocg_form_precedence_direct_off`, `_indirect_off`, `_direct_on`, so the form's group decides whether it is a direct dictionary or an indirect object, and an ON form group under an OFF page group still shows). This addendum changes nothing in the decision.
