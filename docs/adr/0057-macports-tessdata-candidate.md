---
id: 0057
title: Append /opt/local/share/tessdata to the known tessdata list
date: 2026-10-03
status: Accepted
summary: The MacPorts tessdata directory is appended to design-spec section 7.3's known list.
---

# 0057. Append /opt/local/share/tessdata to the known tessdata list

## Context

Spec 0002 D-3. The development machine uses a MacPorts layout that the design spec's list does not name.

## Decision

`/opt/local/share/tessdata` is appended to design-spec section 7.3's known list. The order of the rest is unchanged.

## Alternatives considered

No alternative recorded.

## Consequences

Harmless elsewhere: one `stat`.
