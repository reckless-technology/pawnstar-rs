//! Fixed-capacity stack-allocated list container. (Port of `stack_list.h`.)

use core::mem::MaybeUninit;

/// Fixed size, very simple stack storage container for lists of moves.
/// Considerably faster than a heap `Vec` due to not allocating; more convenient than a raw array
/// for insertion, sorting and iteration. NB: like the C++ original there is no buffer overflow
/// protection beyond a debug assert, so use with caution.
///
/// Like the C++ original the backing array is deliberately left uninitialized (constructing a
/// 256-entry move list per node must not memset 2 KB); only the live prefix `[..len]` is ever
/// read, through the safe accessors below.
pub struct StackList<T: Copy, const N: usize> {
    data: [MaybeUninit<T>; N], // Backing fixed-size storage (only [..len] is initialised).
    len: usize,                // Number of in-use elements (the C++ `end_` iterator).
}

impl<T: Copy, const N: usize> StackList<T, N> {
    /// Construct an empty list. The storage is uninitialised, as in C++.
    #[inline(always)]
    pub fn new() -> Self {
        StackList {
            // SAFETY: an array of MaybeUninit does not require initialization.
            data: unsafe { MaybeUninit::uninit().assume_init() },
            len: 0,
        }
    }

    /// Append an element to the end of the list.
    #[inline(always)]
    pub fn push_back(&mut self, m: T) {
        debug_assert!(self.len < N, "StackList capacity exceeded");
        self.data[self.len].write(m);
        self.len += 1;
    }

    /// Remove and return the last element.
    #[inline(always)]
    pub fn pop_back(&mut self) -> T {
        self.len -= 1;
        // SAFETY: element len (post-decrement) was written by a prior push_back.
        unsafe { self.data[self.len].assume_init() }
    }

    /// Access the last element.
    #[inline(always)]
    pub fn back(&self) -> &T {
        // SAFETY: the last live element was written by push_back.
        unsafe { self.data[self.len - 1].assume_init_ref() }
    }

    /// Access the last element mutably.
    #[inline(always)]
    pub fn back_mut(&mut self) -> &mut T {
        // SAFETY: the last live element was written by push_back.
        unsafe { self.data[self.len - 1].assume_init_mut() }
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
        // SAFETY: elements [..len] were written by push_back; MaybeUninit<T> has T's layout.
        unsafe { core::slice::from_raw_parts(self.data.as_ptr() as *const T, self.len) }
    }

    /// The in-use elements as a mutable slice.
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: elements [..len] were written by push_back; MaybeUninit<T> has T's layout.
        unsafe { core::slice::from_raw_parts_mut(self.data.as_mut_ptr() as *mut T, self.len) }
    }

    /// Iterate over the in-use elements.
    #[inline(always)]
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }
}

impl<T: Copy, const N: usize> Default for StackList<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Copy semantics matching the C++ copy constructor: only the live prefix is copied.
impl<T: Copy, const N: usize> Clone for StackList<T, N> {
    fn clone(&self) -> Self {
        let mut out = Self::new();
        for (i, v) in self.as_slice().iter().enumerate() {
            out.data[i].write(*v);
        }
        out.len = self.len;
        out
    }
}

impl<T: Copy, const N: usize> core::ops::Index<usize> for StackList<T, N> {
    type Output = T;
    #[inline(always)]
    fn index(&self, i: usize) -> &T {
        &self.as_slice()[i]
    }
}

impl<T: Copy, const N: usize> core::ops::IndexMut<usize> for StackList<T, N> {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.as_mut_slice()[i]
    }
}

impl<'a, T: Copy, const N: usize> IntoIterator for &'a StackList<T, N> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;
    #[inline(always)]
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}
