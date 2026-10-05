//! A cumulative memory budget for one decode.
//!
//! JBIG2 sizes most of its buffers from values the file declares, not from
//! the length of the data: a bitmap of up to 65535 × 65535 pixels from a few
//! header bytes, a gray-scale plane of four bytes per grid cell, one clone of
//! a symbol per export, a symbol-ID context table of `2^SBSYMCODELEN`
//! entries. Each is bounded on its own, but nothing bounds their sum, so a
//! stream of a few hundred bytes can ask for hundreds of gigabytes.
//!
//! A [`MemoryBudget`] is created by [`Image::decode_with`](crate::Image::decode_with)
//! from [`DecoderContext::memory_budget`](crate::DecoderContext::memory_budget)
//! and passed by reference to every site that allocates in proportion to a
//! declared value. Each site charges the bytes it is about to allocate
//! *before* allocating them; a charge that would take the running total over
//! the budget fails the decode with
//! [`OverflowError::MemoryBudget`](crate::OverflowError::MemoryBudget). Charges
//! are never refunded when a buffer is dropped, so the budget bounds the total
//! allocated over the decode, which is an upper bound on the peak. Allocations
//! linear in the input (segment headers, referred-to lists, custom Huffman
//! tables) and the fixed-size context tables are not charged.

use core::cell::Cell;

use crate::error::{OverflowError, Result, bail};

/// The bytes left for one decode; `None` means unlimited.
#[derive(Debug)]
pub(crate) struct MemoryBudget {
    remaining: Cell<Option<u64>>,
}

impl MemoryBudget {
    /// A budget of `limit` bytes, or an unlimited one for `None`.
    pub(crate) fn new(limit: Option<u64>) -> Self {
        Self {
            remaining: Cell::new(limit),
        }
    }

    /// Charges `bytes` against the budget, failing without charging anything
    /// when that would exceed it.
    pub(crate) fn charge(&self, bytes: u64) -> Result<()> {
        if let Some(remaining) = self.remaining.get() {
            match remaining.checked_sub(bytes) {
                Some(left) => self.remaining.set(Some(left)),
                None => bail!(OverflowError::MemoryBudget),
            }
        }

        Ok(())
    }

    /// Charges `count` values of type `T`, as a `Vec<T>` of that length or
    /// capacity would allocate.
    pub(crate) fn charge_array<T>(&self, count: u64) -> Result<()> {
        self.charge(count.saturating_mul(size_of::<T>() as u64))
    }

    /// Fails as [`charge`](Self::charge) would for `bytes`, but charges
    /// nothing. Used to refuse a batch of allocations before making the first
    /// one, when each is then charged as it is made.
    pub(crate) fn check(&self, bytes: u64) -> Result<()> {
        match self.remaining.get() {
            Some(remaining) if bytes > remaining => bail!(OverflowError::MemoryBudget),
            _ => Ok(()),
        }
    }
}
