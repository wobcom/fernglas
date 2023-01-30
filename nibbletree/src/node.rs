use std::ops::{Index, IndexMut};
use std::fmt::{Debug, Formatter};
use thin_vec::ThinVec;
use bitvec::prelude::*;

pub type Key = BitVec<usize, Lsb0>;
pub type KeyRef<'a> = &'a BitSlice<usize, Lsb0>;

#[derive(Default)]
struct Bitmap {
    bitmap: BitmapType,
}

type BitmapType = u64;
const RESULTS_BITS_END_NODE: usize = 5;
const RESULTS_BITS: usize = RESULTS_BITS_END_NODE - 1;
const CHILDREN_START_END_NODE: usize = 2_usize.pow(RESULTS_BITS_END_NODE as u32 + 1);
const CHILDREN_START: usize = 2_usize.pow(RESULTS_BITS as u32 + 1);

impl Debug for Bitmap {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let field2_name = if self.is_end_node() { "internal2" } else { "external" };
        f.debug_struct("Bitmap")
            .field("is_end_node", &self.is_end_node())
            .field("internal", &format!("{}", &self.bitmap.view_bits::<Lsb0>()[..CHILDREN_START]))
            .field(field2_name, &format!("{}", &self.bitmap.view_bits::<Lsb0>()[CHILDREN_START..]))
            .finish()
    }
}

impl Bitmap {
    #[inline]
    fn is_end_node(&self) -> bool {
        self.bitmap.view_bits::<Lsb0>()[0]
    }
    fn set_is_end_node(&mut self, is_end_node: bool) {
        self.bitmap.view_bits_mut::<Lsb0>().set(0, is_end_node);
    }
    #[inline]
    fn children_start_at(&self) -> usize {
        if self.is_end_node() { CHILDREN_START_END_NODE } else { CHILDREN_START }
    }
    #[inline]
    fn results_capacity(&self) -> usize {
        if self.is_end_node() { RESULTS_BITS_END_NODE } else { RESULTS_BITS }
    }

    fn children_bits_mut(&mut self) -> &mut BitSlice<BitmapType, Lsb0> {
        let start = self.children_start_at();
        self.bitmap.view_bits_mut::<Lsb0>().index_mut(start..)
    }
    fn children_bits(&self) -> &BitSlice<BitmapType, Lsb0> {
        let start = self.children_start_at();
        self.bitmap.view_bits::<Lsb0>().index(start..)
    }

    fn results_bits_mut(&mut self) -> &mut BitSlice<BitmapType, Lsb0> {
        let end = self.children_start_at();
        self.bitmap.view_bits_mut::<Lsb0>().index_mut(1..end)
    }
    fn results_bits(&self) -> &BitSlice<BitmapType, Lsb0> {
        let end = self.children_start_at();
        self.bitmap.view_bits::<Lsb0>().index(1..end)
    }

    fn results_keys_with_prefix(&self, prefix: Key) -> impl Iterator<Item = Key> + '_ {
        self.results_bits()
            .iter_ones()
            .map(from_index)
            .map(move |result_key| {
                let mut key = prefix.clone();
                key.extend(result_key);
                key
            })
    }
}

#[derive(Debug)]
pub struct Node<T> {
    results: Option<ThinVec<T>>,
    children: Option<ThinVec<Node<T>>>,
    bitmap: Bitmap,
}

impl<T> Default for Node<T> {
    fn default() -> Self {
        let mut bitmap: Bitmap = Default::default();
        bitmap.set_is_end_node(true);
        Node {
            results: None,
            children: None,
            bitmap,
        }
    }
}

fn children_mut<'a, T>(bitmap: &'a Bitmap, children: &'a mut Option<ThinVec<Node<T>>>) -> impl Iterator<Item = (Key, &'a mut Node<T>)> {
    let children_iter = children.iter_mut().flat_map(|children| children.iter_mut());
    bitmap.children_bits().iter_ones().map(|x| x.view_bits::<Lsb0>().iter().take(RESULTS_BITS_END_NODE).collect()).zip(children_iter)
}
fn results_mut<'a, T>(bitmap: &'a Bitmap, results: &'a mut Option<ThinVec<T>>) -> impl Iterator<Item = (Key, &'a mut T)> {
    let results_iter = results.iter_mut().flat_map(|results| results.iter_mut());
    bitmap.results_bits().iter_ones().map(from_index).zip(results_iter)
}

fn to_index(key: KeyRef) -> usize {
    let leading_one = 2usize.pow(key.len() as u32);
    let net_bits: usize = if key.is_empty() { 0 } else { key.load_le() };
    (leading_one + net_bits) - 1
}

fn from_index(mut index: usize) -> Key {
    index += 1;
    let prefix_len = (std::mem::size_of::<usize>() as u32 * 8) - index.leading_zeros() - 1;
    let mut key = Key::new();
    key.extend(index.view_bits::<Lsb0>().iter().take(prefix_len as usize));
    key
}

impl<T: Debug> Node<T> {
    fn children(&self) -> impl Iterator<Item = (Key, &Node<T>)> {
        let children_iter = self.children.iter().flat_map(|children| children.iter());
        self.bitmap.children_bits().iter_ones().map(|x| x.view_bits::<Lsb0>().iter().take(RESULTS_BITS_END_NODE).collect()).zip(children_iter)
    }

    fn results(&self) -> impl Iterator<Item = (Key, &T)> {
        let results_iter = self.results.iter().flat_map(|results| results.iter());
        self.bitmap.results_bits().iter_ones().map(from_index).zip(results_iter)
    }

    fn get_child(&self, key: KeyRef) -> Option<&Node<T>> {
        if self.bitmap.is_end_node() { return None; }
        let nibble: usize = key.load_le();
        self.bitmap.children_bits()[nibble].then(|| {
            let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
            &self.children.as_ref().unwrap()[vec_index]
        })
    }
    fn get_child_mut(&mut self, key: KeyRef) -> Option<&mut Node<T>> {
        if self.bitmap.is_end_node() { return None; }
        let nibble: usize = key.load_le();
        self.bitmap.children_bits()[nibble].then(|| {
            let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
            &mut self.children.as_mut().unwrap()[vec_index]
        })
    }

    fn convert_to_normal(&mut self) {
        if !self.bitmap.is_end_node() { return; }

        let results_iter = self.results.take().into_iter().flat_map(|results| results.into_iter());
        let results = self.bitmap.results_bits().iter_ones().map(from_index).zip(results_iter).collect::<Vec<_>>();

        self.bitmap = Default::default();
        self.bitmap.set_is_end_node(false);

        for (key, value) in results {
            self.insert(&key, value);
        }
    }
    fn get_or_insert_child(&mut self, key: KeyRef) -> &mut Node<T> {
        self.convert_to_normal();

        {
            let nibble: usize = key.load_le();
            if !self.bitmap.children_bits()[nibble] {
                self.bitmap.children_bits_mut().set(nibble, true);
                let children = self.children.get_or_insert(Default::default());
                let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
                children.insert(vec_index, Node::default());
            }
        }
        self.get_child_mut(key).unwrap()
    }

    pub fn insert(&mut self, key: KeyRef, value: T) {
        if key.len() <= self.bitmap.results_capacity() {
            // capacity is suffcient, insert into local node
            let index = to_index(&key);
            self.bitmap.results_bits_mut().set(index, true);
            let results = self.results.get_or_insert(Default::default());
            let vec_index = self.bitmap.results_bits()[..index].count_ones();
            results.insert(vec_index, value);
        } else {
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            // insert into child node
            let child = self.get_or_insert_child(key);
            child.insert(remaining, value);
        }
    }
    pub fn remove(&mut self, key: KeyRef) -> Option<T> {
        if key.len() <= self.bitmap.results_capacity() {
            let index = to_index(key);
            self.bitmap.results_bits()[index].then(|| {
                self.bitmap.results_bits_mut().set(index, false);
                let results = self.results.get_or_insert(Default::default());
                let vec_index = self.bitmap.results_bits()[..index].count_ones();
                results.remove(vec_index)
            })
        } else {
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            self.get_child_mut(key).and_then(|child| child.remove(remaining))
        }
    }

    fn iter_with_prefix(&self, prefix: Key) -> impl Iterator<Item = (Key, &T)> + '_ {
        let results_iter = {
            let prefix = prefix.clone();
            self.results().map(move |(result_key, val)| {
                let mut key = prefix.clone();
                key.extend(result_key);
                (key, val)
            })
        };
        let children_iter = self.children()
            .flat_map(move |(child_key, child)| {
                let mut key = prefix.clone();
                key.extend(child_key);
                child.iter_with_prefix(key)
            });
        let children_iter: Box<dyn Iterator<Item = (Key, &T)> + '_>  = Box::new(children_iter);
        results_iter.chain(children_iter)
    }
    fn iter_mut_with_prefix(&mut self, prefix: Key) -> impl Iterator<Item = (Key, &mut T)> + '_ {
        let results_iter = {
            let prefix = prefix.clone();
            results_mut(&self.bitmap, &mut self.results).map(move |(result_key, val)| {
                let mut key = prefix.clone();
                key.extend(result_key);
                (key, val)
            })
        };
        let children_iter = children_mut(&self.bitmap, &mut self.children)
            .flat_map(move |(child_key, child)| {
                let mut key = prefix.clone();
                key.extend(child_key);
                child.iter_mut_with_prefix(key)
            });
        let children_iter: Box<dyn Iterator<Item = (Key, &mut T)> + '_>  = Box::new(children_iter);
        results_iter.chain(children_iter)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Key, &T)> + '_ {
        self.iter_with_prefix(Key::new())
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Key, &mut T)> + '_ {
        self.iter_mut_with_prefix(Key::new())
    }

    pub fn values(&self) -> impl Iterator<Item = &T> + '_ {
        let results_iter = self.results.iter().flat_map(|values| values.iter());
        let children_iter = self.children.iter()
            .flat_map(|children| children.iter())
            .flat_map(|child| child.values());
        let children_iter: Box<dyn Iterator<Item = &T> + '_> = Box::new(children_iter);
        results_iter.chain(children_iter)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        let results_iter = self.results.iter_mut().flat_map(|values| values.iter_mut());
        let children_iter = self.children.iter_mut()
            .flat_map(|children| children.iter_mut())
            .flat_map(|child| child.values_mut());
        let children_iter: Box<dyn Iterator<Item = &mut T> + '_> = Box::new(children_iter);
        results_iter.chain(children_iter)
    }

    fn keys_with_prefix<'a>(&'a self, prefix: Key) -> impl Iterator<Item = Key> + '_ {
        let results_keys_iter = self.bitmap.results_keys_with_prefix(prefix.clone());
        let children_keys_iter = self.children()
            .flat_map(move |(child_key, child)| {
                let mut key = prefix.clone();
                key.extend(child_key);
                child.keys_with_prefix(key)
            });
        let children_keys_iter: Box<dyn Iterator<Item = Key> + '_> = Box::new(children_keys_iter);
        results_keys_iter.chain(children_keys_iter)
    }

    pub fn keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.keys_with_prefix(Key::new())
    }

    pub fn exact(&self, key: KeyRef) -> Option<&T> {
        if key.len() <= self.bitmap.results_capacity() {
            let index = to_index(key);
            self.bitmap.results_bits()[index].then(|| {
                let vec_index = self.bitmap.results_bits()[..index].count_ones();
                &self.results.as_ref().unwrap()[vec_index]
            })
        } else {
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            self.get_child(key).and_then(|child| child.exact(remaining))
        }
    }
    pub fn exact_mut(&mut self, key: KeyRef) -> Option<&mut T> {
        if key.len() <= self.bitmap.results_capacity() {
            let index = to_index(key);
            self.bitmap.results_bits()[index].then(|| {
                let vec_index = self.bitmap.results_bits()[..index].count_ones();
                &mut self.results.as_mut().unwrap()[vec_index]
            })
        } else {
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            self.get_child_mut(key).and_then(|child| child.exact_mut(remaining))
        }
    }

    fn longest_match_with_prefix(&self, mut prefix: Key, mut key: KeyRef) -> Option<(Key, &T)> {
        (key.len() > self.bitmap.results_capacity()).then(|| {
            let mut prefix = prefix.clone();
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            prefix.extend(key);
            self.get_child(key).and_then(|child| child.longest_match_with_prefix(prefix, remaining))
        })
        .flatten()
        .or_else(|| {
            loop {
                if let Some(result) = self.exact(key.clone()) {
                    prefix.extend(key);
                    return Some((prefix, result));
                }
                if !key.is_empty() {
                    key = &key[..key.len() - 1];
                } else {
                    break;
                }
            }
            None
        })
    }
    pub fn longest_match(&self, key: KeyRef) -> Option<(Key, &T)> {
        self.longest_match_with_prefix(Key::new(), key)
    }

    fn or_longer_with_prefix(&self, prefix: Key, mut key: Key) -> Box<dyn Iterator<Item = (Key, &T)> + '_> {
        if key.len() > self.bitmap.results_capacity() {
            let mut prefix = prefix.clone();
            let remaining = key.split_off(RESULTS_BITS_END_NODE);
            prefix.extend(&key);
            if let Some(child) = self.get_child(&key) {
                child.or_longer_with_prefix(prefix, remaining)
            } else {
                Box::new(std::iter::empty())
            }
        } else {
            let results_iter = {
                let prefix = prefix.clone();
                let key = key.clone();
                self.results()
                    .filter(move |(result_key, _)| {
                        result_key.starts_with(&key)
                    })
                    .map(move |(result_key, val)| {
                        let mut key = prefix.clone();
                        key.extend(result_key);
                        (key, val)
                    })
            };
            let children_iter = self.children()
                .filter(move |(child_key, _)| {
                    child_key.starts_with(&key)
                })
                .flat_map(move |(child_key, child)| {
                    let mut key = prefix.clone();
                    key.extend(child_key);
                    child.iter_with_prefix(key)
                });
            let children_iter: Box<dyn Iterator<Item = (Key, &T)> + '_>  = Box::new(children_iter);
            Box::new(results_iter.chain(children_iter))
        }
    }
    pub fn or_longer(&self, key: Key) -> impl Iterator<Item = (Key, &T)> + '_ {
        self.or_longer_with_prefix(Key::new(), key)
    }

    fn matches_with_prefix(&self, prefix: Key, mut key: Key) -> impl Iterator<Item = (Key, &T)> + '_ {
        let results_iter = {
            let prefix = prefix.clone();
            let key = key.clone();
            self.results()
                .filter(move |(result_key, _)| {
                    key.starts_with(result_key)
                })
                .map(move |(result_key, val)| {
                    let mut key = prefix.clone();
                    key.extend(result_key);
                    (key, val)
                })
        };
        let children_iter = {
            let key = key.clone();
            self.children()
                .filter(move |(child_key, _)| {
                    key.starts_with(child_key)
                })
        };
        let children_iter = children_iter
            .flat_map(move |(child_key, child)| {
                let remaining = key.split_off(RESULTS_BITS_END_NODE);

                let mut key = prefix.clone();
                key.extend(child_key);
                child.matches_with_prefix(key, remaining)
            });
        let children_iter: Box<dyn Iterator<Item = (Key, &T)> + '_>  = Box::new(children_iter);
        results_iter.chain(children_iter)
    }
    pub fn matches(&self, key: Key) -> impl Iterator<Item = (Key, &T)> + '_ {
        self.matches_with_prefix(Key::new(), key)
    }
}
