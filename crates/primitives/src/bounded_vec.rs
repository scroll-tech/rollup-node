//! A bounded vector implementation.

use core::ops::{Deref, RangeBounds};
use std::collections::{vec_deque::Drain, VecDeque};

/// A bounded vec implementation using [`VecDeque`]. The structure will overwrite the oldest data
/// that was pushed in when it reaches capacity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoundedVec<T> {
    data: VecDeque<T>,
}

impl<T> BoundedVec<T> {
    /// Returns a new instance of the ring buffer with the provided capacity.
    pub fn new(capacity: usize) -> Self {
        Self { data: VecDeque::with_capacity(capacity) }
    }

    /// Pushes a value at the back of the buffer. If the buffer is full,
    pub fn push(&mut self, elem: T) {
        if self.is_full() {
            self.data.pop_front();
        }

        self.data.push_back(elem)
    }

    /// Pops the last element from the structure and returns it if any.
    pub fn pop(&mut self) -> Option<T> {
        self.data.pop_back()
    }

    /// Returns the last element in the vector, if any.
    pub fn last(&self) -> Option<&T> {
        self.data.back()
    }

    /// Clears the structure by removing all the elements.
    pub fn clear(&mut self) {
        self.data.clear()
    }

    /// Drains the structure by removing the provided range of elements.
    pub fn drain<R>(&mut self, range: R) -> Drain<'_, T>
    where
        R: RangeBounds<usize>,
    {
        self.data.drain(range)
    }

    #[inline]
    fn is_full(&self) -> bool {
        self.data.len() == self.data.capacity()
    }
}

impl<T> Extend<T> for BoundedVec<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        let iter = iter.into_iter();

        // if size hint returns an upper bound, skip values until whole iterator can fit in the
        // bounded vec.
        let mut iter = if let (_, Some(upper_bound)) = iter.size_hint() {
            iter.skip(upper_bound.saturating_sub(self.data.capacity()))
        } else {
            iter.skip(0)
        };

        while let Some(elem) = iter.next() {
            self.push(elem)
        }
    }
}

impl<T> From<Vec<T>> for BoundedVec<T> {
    fn from(value: Vec<T>) -> Self {
        // the final bounded vec will have twice the capacity as the vec.
        let mut bounded = Self::new(0);
        let capacity = value.capacity();
        bounded.data = value.into();
        bounded.data.reserve_exact(capacity);
        bounded
    }
}

impl<T: PartialEq> PartialEq<Vec<T>> for BoundedVec<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.data == other.as_slice()
    }
}

impl<T> Deref for BoundedVec<T> {
    type Target = VecDeque<T>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
