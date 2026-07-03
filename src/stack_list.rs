//! Fixed-capacity stack-allocated list container. (Port of `stack_list.h`.)

/// Fixed size, very simple stack storage container for lists of moves.
/// Considerably faster than a heap `Vec` due to not allocating; more convenient than a raw array
/// for insertion, sorting and iteration. NB: like the C++ original there is no buffer overflow
/// protection beyond a debug assert, so use with caution.
///
/// The C++ original leaves the backing array uninitialized; here the array is value-initialized
/// via `Default` (a deliberate porting choice — a `MaybeUninit` no-init version is a later,
/// parity-locked perf change).
pub struct StackList<T: Copy + Default, const N: usize> {
    data: [T; N], // Backing fixed-size storage.
    len: usize,   // Number of in-use elements (the C++ `end_` iterator).
}

impl<T: Copy + Default, const N: usize> StackList<T, N> {
    /// Construct an empty list.
    #[inline]
    pub fn new() -> Self {
        StackList {
            data: [T::default(); N],
            len: 0,
        }
    }

    /// Append an element to the end of the list.
    #[inline(always)]
    pub fn push_back(&mut self, m: T) {
        debug_assert!(self.len < N, "StackList capacity exceeded");
        self.data[self.len] = m;
        self.len += 1;
    }

    /// Remove and return the last element.
    #[inline(always)]
    pub fn pop_back(&mut self) -> T {
        self.len -= 1;
        self.data[self.len]
    }

    /// Access the last element.
    #[inline(always)]
    pub fn back(&self) -> &T {
        &self.data[self.len - 1]
    }

    /// Access the last element mutably.
    #[inline(always)]
    pub fn back_mut(&mut self) -> &mut T {
        &mut self.data[self.len - 1]
    }

    /// Remove all elements.
    #[inline(always)]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Number of elements currently stored.
    #[inline(always)]
    pub fn size(&self) -> usize {
        self.len
    }

    /// Whether the list is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The in-use elements as a slice (the C++ begin()/end() range).
    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        &self.data[..self.len]
    }

    /// The in-use elements as a mutable slice.
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data[..self.len]
    }

    /// Iterate over the in-use elements.
    #[inline(always)]
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }
}

impl<T: Copy + Default, const N: usize> Default for StackList<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Copy semantics matching the C++ copy constructor: only the live prefix is copied.
impl<T: Copy + Default, const N: usize> Clone for StackList<T, N> {
    fn clone(&self) -> Self {
        let mut out = Self::new();
        out.data[..self.len].copy_from_slice(&self.data[..self.len]);
        out.len = self.len;
        out
    }
}

impl<T: Copy + Default, const N: usize> core::ops::Index<usize> for StackList<T, N> {
    type Output = T;
    #[inline(always)]
    fn index(&self, i: usize) -> &T {
        &self.data[i]
    }
}

impl<T: Copy + Default, const N: usize> core::ops::IndexMut<usize> for StackList<T, N> {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.data[i]
    }
}

impl<'a, T: Copy + Default, const N: usize> IntoIterator for &'a StackList<T, N> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;
    #[inline(always)]
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}
