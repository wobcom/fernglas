use std::cmp::Ordering;

pub struct Diff<L: Iterator, R: Iterator, LF, RF> {
    left: std::iter::Peekable<L>,
    right: std::iter::Peekable<R>,
    left_key_fn: LF,
    right_key_fn: RF,
}

pub enum DiffEvent<LI, RI> {
    OnlyLeft(LI),
    OnlyRight(RI),
    Both(LI, RI),
}

impl<L, R, LF, RF, LI, RI, U> Iterator for Diff<L, R, LF, RF>
where
    L: Iterator<Item = LI>,
    R: Iterator<Item = RI>,
    LF: Fn(&LI) -> U,
    RF: Fn(&RI) -> U,
    U: Ord,
{
    type Item = DiffEvent<LI, RI>;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.left.peek(), self.right.peek()) {
            (Some(l), Some(r)) => match (self.left_key_fn)(l).cmp(&(self.right_key_fn)(r)) {
                Ordering::Less => Some(DiffEvent::OnlyLeft(self.left.next().unwrap())),
                Ordering::Greater => Some(DiffEvent::OnlyRight(self.right.next().unwrap())),
                Ordering::Equal => Some(DiffEvent::Both(
                    self.left.next().unwrap(),
                    self.right.next().unwrap(),
                )),
            },
            (Some(_), None) => Some(DiffEvent::OnlyLeft(self.left.next().unwrap())),
            (None, Some(_)) => Some(DiffEvent::OnlyRight(self.right.next().unwrap())),
            (None, None) => None,
        }
    }
}

impl<L: Iterator, R: Iterator, LF, RF> Diff<L, R, LF, RF> {
    pub fn new<LS, RS>(left: LS, right: RS, left_key_fn: LF, right_key_fn: RF) -> Self
    where
        LS: IntoIterator<IntoIter = L>,
        RS: IntoIterator<IntoIter = R>,
    {
        Diff {
            left: left.into_iter().peekable(),
            right: right.into_iter().peekable(),
            left_key_fn,
            right_key_fn,
        }
    }
}
