//! A byte reader.

use core::ops::Range;

/// The maximum nesting depth of arrays and dictionaries that the object parser
/// accepts.
///
/// Arrays and dictionaries are parsed recursively, so without a cap a crafted
/// file (for example 200,000 nested `[`) overflows the stack and aborts the
/// process. Real documents nest a handful of levels; an object that nests
/// deeper than this fails to parse, as any other malformed object does, and
/// [`object_depth_limit_hits`] records that it happened.
pub const MAX_OBJECT_DEPTH: u32 = 256;

#[cfg(feature = "std")]
std::thread_local! {
    // Thread-local rather than per-document: content streams are parsed with
    // no reference to their document, so this is the one place every parse
    // site can report to. A `Cell` makes the hot path a plain load and store.
    static OBJECT_DEPTH_LIMIT_HITS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Return how many times, on the current thread, a parse was refused because
/// it would have nested deeper than [`MAX_OBJECT_DEPTH`].
///
/// The counter only ever increases. To learn whether an operation ran into the
/// limit, read it before and after the operation, on the same thread. Without
/// the `std` feature nothing is counted and this always returns 0.
pub fn object_depth_limit_hits() -> u64 {
    #[cfg(feature = "std")]
    {
        OBJECT_DEPTH_LIMIT_HITS.with(core::cell::Cell::get)
    }

    #[cfg(not(feature = "std"))]
    {
        0
    }
}

/// Record that a parse was refused because of a nesting limit.
///
/// Called by this crate when [`MAX_OBJECT_DEPTH`] is reached; public so that
/// parsers built on top of it (such as the PostScript calculator functions in
/// `hayro-interpret`) report their own nesting limits through the same
/// counter, see [`object_depth_limit_hits`].
pub fn note_object_depth_limit_hit() {
    #[cfg(feature = "std")]
    OBJECT_DEPTH_LIMIT_HITS.with(|hits| hits.set(hits.get().saturating_add(1)));
}

/// A reader for reading bytes and PDF objects.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    /// The underlying data of the reader.
    pub data: &'a [u8],
    /// The current byte-offset.
    pub offset: usize,
    /// How many arrays and dictionaries are currently being parsed, one inside
    /// the other. Bounded by [`MAX_OBJECT_DEPTH`].
    depth: u32,
}

impl<'a> Reader<'a> {
    /// Create a new reader.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self::new_with(data, 0)
    }

    /// Create a new reader at the given offset.
    #[inline]
    pub fn new_with(data: &'a [u8], offset: usize) -> Self {
        Self {
            data,
            offset,
            depth: 0,
        }
    }

    /// Run `f`, which parses one array or dictionary, one nesting level deeper.
    ///
    /// Returns `None` without calling `f` if that level would exceed
    /// [`MAX_OBJECT_DEPTH`], and records the refusal (see
    /// [`object_depth_limit_hits`]).
    #[inline]
    pub(crate) fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        if self.depth >= MAX_OBJECT_DEPTH {
            warn!("object nesting exceeds the maximum depth of {MAX_OBJECT_DEPTH}");
            note_object_depth_limit_hit();

            return None;
        }

        self.depth += 1;
        let result = f(self);
        self.depth -= 1;

        result
    }

    /// Returns `true` if the reader has reached the end of the data.
    #[inline]
    pub fn at_end(&self) -> bool {
        self.offset >= self.data.len()
    }

    /// Moves the reader offset to the end of the data.
    #[inline]
    pub fn jump_to_end(&mut self) {
        self.offset = self.data.len();
    }

    /// Moves the reader to the specified offset.
    #[inline]
    pub fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    /// Returns the remaining data from the current offset to the end.
    #[inline]
    pub fn tail(&mut self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

    /// Returns the total length of the underlying data.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the underlying data is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns a slice of the data for the specified range.
    #[inline]
    pub fn range(&self, range: Range<usize>) -> Option<&'a [u8]> {
        self.data.get(range)
    }

    /// Returns the current offset of the reader.
    #[inline]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Reads the specified number of bytes and advances the offset.
    #[inline]
    pub fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let v = self.peek_bytes(len)?;
        self.offset += len;

        Some(v)
    }

    /// Reads a single byte and advances the offset.
    #[inline]
    pub fn read_byte(&mut self) -> Option<u8> {
        let v = self.peek_byte()?;
        self.offset += 1;

        Some(v)
    }

    /// Skips the specified number of bytes by advancing the offset.
    #[inline]
    pub fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| {})
    }

    /// Peeks the specified number of bytes.
    #[inline]
    pub fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        self.offset
            .checked_add(len)
            .and_then(|end| self.data.get(self.offset..end))
    }

    /// Peeks a single byte.
    #[inline]
    pub fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.offset).copied()
    }

    /// Eat the next byte if it satisfies the condition.
    #[inline]
    pub fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        let val = self.peek_byte()?;
        if f(val) {
            self.forward();
            Some(val)
        } else {
            None
        }
    }

    /// Advances the offset by one byte.
    #[inline]
    pub fn forward(&mut self) {
        self.offset += 1;
    }

    /// Advances the offset by one byte if the current byte satisfies the predicate.
    #[inline]
    pub fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        if f(self.peek_byte()?) {
            self.forward();

            Some(())
        } else {
            None
        }
    }

    /// Advances the offset while bytes satisfy the predicate, at least one time.
    #[inline]
    pub fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        self.eat(&f)?;
        self.forward_while(f);
        Some(())
    }

    /// Advances the offset if the next bytes match the specified tag.
    #[inline]
    pub fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        self.peek_tag(tag)?;
        self.offset += tag.len();

        Some(())
    }

    /// Advances the offset while the given byte satisfies the predicate.
    #[inline]
    pub fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        while let Some(b) = self.peek_byte() {
            if f(b) {
                self.forward();
            } else {
                break;
            }
        }
    }

    /// Checks if the next bytes match the specified tag.
    #[inline]
    pub fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        let mut cloned = self.clone();

        for b in tag.iter().copied() {
            if cloned.peek_byte() == Some(b) {
                cloned.forward();
            } else {
                return None;
            }
        }

        Some(())
    }

    /// Read a u16 integer (in big endian order).
    #[inline]
    pub fn read_u16(&mut self) -> Option<u16> {
        let bytes = self.read_bytes(2)?;

        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Read a u32 integer (in big endian order).
    #[inline]
    pub fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read_bytes(4)?;

        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a u64 integer (in big endian order).
    #[inline]
    pub fn read_u64(&mut self) -> Option<u64> {
        let bytes = self.read_bytes(8)?;

        Some(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::Reader;

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let reader = Reader::new(b"abc");
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }
}
