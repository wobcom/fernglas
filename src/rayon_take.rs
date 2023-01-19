use rayon::prelude::ParallelIterator;
use rayon::iter::plumbing::UnindexedConsumer;
use rayon::iter::plumbing::Consumer;
use rayon::iter::plumbing::Folder;
use std::sync::atomic::{AtomicUsize, Ordering};

#[must_use = "iterator adaptors are lazy and do nothing unless consumed"]
#[derive(Debug)]
pub struct Take2<I: ParallelIterator> {
    base: I,
    count: AtomicUsize,
}

impl<I> Take2<I>
where
    I: ParallelIterator,
{
    /// Creates a new `Take2` iterator.
    pub(super) fn new(base: I, count: usize) -> Self {
        Take2 {
            base,
            count: AtomicUsize::new(count)
        }
    }
}

impl<I, T> ParallelIterator for Take2<I>
where
    I: ParallelIterator<Item = T>,
    T: Send,
{
    type Item = T;

    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        let consumer1 = Take2Consumer {
            base: consumer,
            count: &self.count,
        };
        self.base.drive_unindexed(consumer1)
    }
}

/// ////////////////////////////////////////////////////////////////////////
/// Consumer implementation

struct Take2Consumer<'f, C> {
    base: C,
    count: &'f AtomicUsize,
}

impl<'f, T, C> Consumer<T> for Take2Consumer<'f, C>
where
    C: Consumer<T>,
    T: Send,
{
    type Folder = Take2Folder<'f, C::Folder>;
    type Reducer = C::Reducer;
    type Result = C::Result;

    fn split_at(self, index: usize) -> (Self, Self, Self::Reducer) {
        let (left, right, reducer) = self.base.split_at(index);
        (
            Take2Consumer { base: left, ..self },
            Take2Consumer {
                base: right,
                ..self
            },
            reducer,
        )
    }

    fn into_folder(self) -> Self::Folder {
        Take2Folder {
            base: self.base.into_folder(),
            count: self.count,
        }
    }

    fn full(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0 || self.base.full()
    }
}

impl<'f, T, C> UnindexedConsumer<T> for Take2Consumer<'f, C>
where
    C: UnindexedConsumer<T>,
    T: Send,
{
    fn split_off_left(&self) -> Self {
        Take2Consumer {
            base: self.base.split_off_left(),
            ..*self
        }
    }

    fn to_reducer(&self) -> Self::Reducer {
        self.base.to_reducer()
    }
}

struct Take2Folder<'f, C> {
    base: C,
    count: &'f AtomicUsize,
}

fn checked_decrement(u: &AtomicUsize) -> bool {
        let mut prev = u.load(Ordering::Relaxed);
        while let Some(next) = prev.checked_sub(1) {
            match u.compare_exchange(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return true,
                Err(next_prev) => prev = next_prev,
            }
        }
        return false
}

impl<'f, T, C> Folder<T> for Take2Folder<'f, C>
where
    C: Folder<T>,
{
    type Result = C::Result;

    fn consume(mut self, item: T) -> Self {
        if checked_decrement(self.count) {
            self.base = self.base.consume(item);
        }
        self
    }

    fn consume_iter<I>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = T>,
    {
        self.base = self.base.consume_iter(
            iter.into_iter()
                .take_while(move |_| {
                    checked_decrement(self.count)
                })
        );
        self
    }

    fn complete(self) -> C::Result {
        self.base.complete()
    }

    fn full(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0 || self.base.full()
    }
}


pub trait ParallelIteratorExt: ParallelIterator {
    fn take2(self, count: usize) -> Take2<Self>;
}
impl<I> ParallelIteratorExt for I where I: ParallelIterator {
    fn take2(self, count: usize) -> Take2<I> {
        Take2::new(self, count)
    }
}
