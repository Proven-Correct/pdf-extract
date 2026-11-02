---
id: 0033
title: The character cap cancels interpretation and returns the partial page
date: 2026-10-01
status: Accepted
summary: Hitting max_chars_per_page raises the cancel flag, stops hayro and returns the page so far; an earlier budget exhaustion stays an error.
---

# 0033. The character cap cancels interpretation and returns the partial page

## Context

Decision D-38 (revision 6). Requirement T34 promised that the 2M characters already collected are returned.

## Decision

The text device raises the H10g cancel flag at the character cap; hayro stops within `check_interval` operators
(`BudgetExhaustion::Cancelled`) and the partial page is returned with the warning `text: character limit reached`. A
budget that ran out before the cap stays `PageError::Limit`.

## Alternatives considered

Letting the page run on to a later error was rejected: cancelling stops the CPU spend (R4).

## Consequences

No further operator or time budget is consumed after the cap.
