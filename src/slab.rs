#![allow(dead_code)]

/// A vector tracked by a backing bitmap. You could call this a primitive
/// allocator for uniform structures that supports in-place free.
pub struct Slab<T> {
    /// The inner backing memory
    inner: Vec<T>,

    /// A bitmap of the `inner` vector.
    ///
    /// It follows this format; given `[ 11111111, 11011101, 0000000 ]`,
    /// the first 8 entries in `inner` (first octet of this bitmap) is full.
    /// The second 8 entries in `inner` show that `inner[9]` and `inner[13]`
    /// are empty. `inner[16..]` is empty.
    bitmap: Vec<usize>,
}

impl<T> Slab<T> {
    /// Create a new slab of the capacity `cap`
    pub fn with_capacity(cap: usize) -> Self {
        let bm_entries = cap / usize::BITS as usize + 1;

        Self {
            inner:  Vec::with_capacity(cap),
            bitmap: vec![0usize; bm_entries],
        }
    }

    /// Insert an `element` into the vector and return its index.
    pub fn insert(&mut self, element: T) -> usize {
        let idx = if let Some(idx) = self.get_free() {
            // If we found a free spot, insert the element into it
            self.inner[idx] = element;
            idx
        } else {
            // If we couldn't find one, allocate a new one.
            let idx = self.inner.len();
            self.inner.push(element);

            // Make sure the bitmap has enough bits to work with
            if idx == (self.bitmap.len() * usize::BITS as usize) {
                self.bitmap.push(1);
            }
            idx
        };

        // Mark the bit as vacant
        self.mark_vacant(idx);
        idx
    }

    /// Returns a reference to an element if present
    pub fn get(&self, idx: usize) -> Option<&T> {
        if self.is_vacant(idx) { Some(&self.inner[idx]) } else { None }
    }

    /// Returns a mutable reference to an element if present
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        if self.is_vacant(idx) { Some(&mut self.inner[idx]) } else { None }
    }

    /// Marks the element at `idx` as free
    pub fn mark_free(&mut self, idx: usize) {
        let (map_idx, bit_idx) = self.get_bitmap_idx(idx);
        self.bitmap[map_idx] &= !(1 << bit_idx);
    }

    /// Marks the element at `idx` as vacant
    fn mark_vacant(&mut self, idx: usize) {
        let (map_idx, bit_idx) = self.get_bitmap_idx(idx);
        self.bitmap[map_idx] |= 1 << bit_idx;
    }

    /// Returns the index of the first uninhabited (free) spot in the vector.
    fn get_free(&self) -> Option<usize> {
        self.bitmap.iter().enumerate().find(|(_idx, bm)| {
            // Try and find a free spot
            bm.trailing_ones() != usize::BITS
        })
            // Convert the bit to an index into `self.inner`
            .map(|(idx, bm)| {
                idx * usize::BITS as usize + bm.trailing_ones() as usize
            })

            // If the index is past the end of `self.inner`, it's not allocated
            .filter(|&idx| { idx < self.inner.len() })
    }

    /// Checks whether the element at `idx` in the inner vector is vacant.
    ///
    /// Panics if `idx` is larger than the length of the allocated inner vector.
    fn is_vacant(&self, idx: usize) -> bool {
        assert!(idx < self.inner.len());
        let (map_idx, bit_idx) = self.get_bitmap_idx(idx);

        (self.bitmap[map_idx] & (1 << bit_idx)) != 0
    }

    /// Given an `idx` into the inner vector, return the index into the inner
    /// bitmap vector, and the position of the bit corresponding to the idx,
    /// respectively
    fn get_bitmap_idx(&self, idx: usize) -> (usize, usize) {
        let bit_idx = idx % usize::BITS as usize;
        let map_idx = idx / usize::BITS as usize;
        (map_idx, bit_idx)
    }
}
