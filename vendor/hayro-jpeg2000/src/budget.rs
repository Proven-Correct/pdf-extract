//! A cumulative memory budget for one decode.
//!
//! JPEG 2000 sizes most of its working set from values the codestream
//! declares rather than from the length of its data: one tile record per
//! tile of the reference grid (which component subsampling makes up to
//! 255² times larger than the image), one channel plane per component, one
//! code-block, two tag trees and a run of layer records per precinct, all
//! from a few header bytes. Each is bounded on its own, but nothing bounds
//! their sum, so a codestream of a few hundred bytes can ask for terabytes.
//!
//! A [`MemoryBudget`] is created from
//! [`DecodeSettings::max_memory_bytes`](crate::DecodeSettings::max_memory_bytes)
//! at the start of every decode and lives in the
//! [`DecoderContext`](crate::DecoderContext) for that decode; header parsing
//! uses a budget of its own, so that `Image::new` refuses a header whose tile
//! records alone exceed the limit. Each site that allocates in proportion to
//! a declared value charges the bytes it is about to allocate *before*
//! allocating them; a charge that would take the running total over the
//! budget fails with
//! [`ValidationError::MemoryBudget`](crate::ValidationError::MemoryBudget).
//! Charges are never refunded, so the budget bounds the total allocated over
//! the decode, which is an upper bound on the peak.
//!
//! Buffers the decoder reuses across tiles and code-blocks (the coefficient
//! buffer, the IDWT buffers, the code-block decode buffers and the structural
//! record lists, which are cleared per tile but keep their capacity) grow
//! through [`MemoryBudget::reserve`] and [`MemoryBudget::reserve_exact`],
//! which charge exactly the capacity they add and add it themselves, so
//! `Vec`'s own amortised growth never allocates past what was charged and
//! reusing capacity costs nothing. Allocations linear in the input (marker
//! and box contents, tile-part and segment lists, packet data) are not
//! charged.

use alloc::vec::Vec;
use core::cell::Cell;

use crate::error::{Result, ValidationError, bail};

/// The bytes left for one decode; `None` means unlimited.
#[derive(Debug, Default)]
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
                None => bail!(ValidationError::MemoryBudget),
            }
        }

        Ok(())
    }

    /// Charges `count` values of type `T`, as a `Vec<T>` of that length or
    /// capacity would allocate.
    pub(crate) fn charge_array<T>(&self, count: u64) -> Result<()> {
        self.charge(count.saturating_mul(size_of::<T>() as u64))
    }

    /// Whether `bytes` more would still be within the budget.
    fn fits(&self, bytes: u64) -> bool {
        self.remaining
            .get()
            .is_none_or(|remaining| bytes <= remaining)
    }

    /// Makes room in `vec` for `len` values, growing it to exactly the
    /// capacity charged: at least double its capacity, as `Vec` itself
    /// would, so that many small reservations stay linear, or just `len`
    /// when doubling would not fit the budget but `len` does. Nothing is
    /// charged while the capacity suffices.
    pub(crate) fn reserve<T>(&self, vec: &mut Vec<T>, len: u64) -> Result<()> {
        let capacity = vec.capacity() as u64;
        if len <= capacity {
            return Ok(());
        }

        let doubled = len.max(capacity.saturating_mul(2));
        let size = size_of::<T>() as u64;
        let target = if self.fits((doubled - capacity).saturating_mul(size)) {
            doubled
        } else {
            len
        };
        self.grow_to(vec, target)
    }

    /// [`MemoryBudget::reserve`] for `additional` values more than `vec`
    /// holds now.
    pub(crate) fn reserve_more<T>(&self, vec: &mut Vec<T>, additional: u64) -> Result<()> {
        let len = (vec.len() as u64).saturating_add(additional);
        self.reserve(vec, len)
    }

    /// Makes room in `vec` for exactly `len` values, charging the capacity
    /// added. Nothing is charged while the capacity suffices.
    pub(crate) fn reserve_exact<T>(&self, vec: &mut Vec<T>, len: u64) -> Result<()> {
        if len <= vec.capacity() as u64 {
            return Ok(());
        }

        self.grow_to(vec, len)
    }

    /// Charges and reserves `vec`'s growth from its capacity to `target`.
    fn grow_to<T>(&self, vec: &mut Vec<T>, target: u64) -> Result<()> {
        self.charge_array::<T>(target - vec.capacity() as u64)?;
        // Only a target beyond the address space (no budget, a 32-bit
        // target) fails here; refusing it is kinder than the
        // capacity-overflow panic `reserve_exact` would raise.
        let target = usize::try_from(target).map_err(|_| ValidationError::ImageTooLarge)?;
        vec.reserve_exact(target - vec.len());
        Ok(())
    }
}
