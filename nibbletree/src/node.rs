use bitvec::prelude::*;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::ops::{Index, IndexMut};
use std::sync::LazyLock;
use std::sync::Mutex;
use thin_vec::ThinVec;

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
        let field2_name = if self.is_end_node() {
            "internal2"
        } else {
            "external"
        };
        f.debug_struct("Bitmap")
            .field("is_end_node", &self.is_end_node())
            .field(
                "internal",
                &format!("{}", &self.bitmap.view_bits::<Lsb0>()[..CHILDREN_START]),
            )
            .field(
                field2_name,
                &format!("{}", &self.bitmap.view_bits::<Lsb0>()[CHILDREN_START..]),
            )
            .finish()
    }
}

fn all_possible_keys(max_bits: usize) -> Box<dyn Iterator<Item = Key>> {
    if max_bits == 0 {
        Box::new([Key::new()].into_iter())
    } else {
        Box::new(all_possible_keys(max_bits - 1).flat_map(move |end| {
            let mut res = vec![end.clone()];
            if end.len() == max_bits - 1 {
                res.extend([false, true].into_iter().map(move |begin_value| {
                    let mut key = Key::new();
                    key.extend(end.clone());
                    key.push(begin_value);
                    key
                }));
            }
            res
        }))
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
        if self.is_end_node() {
            CHILDREN_START_END_NODE
        } else {
            CHILDREN_START
        }
    }
    #[inline]
    fn results_capacity(&self) -> usize {
        if self.is_end_node() {
            RESULTS_BITS_END_NODE
        } else {
            RESULTS_BITS
        }
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

    fn to_index(&self, key: KeyRef) -> usize {
        let results_capacity = self.results_capacity();
        key.iter()
            .enumerate()
            .map(|(pos, i)| {
                let val = i
                    .then(|| 2_usize.pow((results_capacity - pos) as u32) - 1)
                    .unwrap_or(0);
                val + 1
            })
            .sum()
    }

    fn from_index(&self, index: usize) -> Key {
        let results_capacity = self.results_capacity();
        static FROM_INDEX_LOOKUP_TABLES: LazyLock<Mutex<HashMap<usize, Vec<Key>>>> =
            LazyLock::new(Default::default);
        let mut lookup_tables = FROM_INDEX_LOOKUP_TABLES.lock().unwrap();
        let lookup_table = lookup_tables.entry(results_capacity).or_insert_with(|| {
            let mut results: Vec<Option<Key>> = (1..2_usize.pow((results_capacity + 1) as u32))
                .map(|_| None)
                .collect();
            for key in all_possible_keys(results_capacity) {
                let index = self.to_index(&key);
                if results[index].is_some() {
                    panic!()
                }
                results[index] = Some(key);
            }
            results.into_iter().map(|x| x.unwrap()).collect()
        });
        lookup_table[index].clone()
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

fn results_and_children_mut<'a, T>(
    bitmap: &'a Bitmap,
    results: &'a mut Option<ThinVec<T>>,
    children: &'a mut Option<ThinVec<Node<T>>>,
) -> impl Iterator<Item = (Key, ResultOrChildMut<'a, T>)> {
    let mut children_values_iter = children.iter_mut().flat_map(|children| children.iter_mut());
    let mut children_iter = bitmap
        .children_bits()
        .iter()
        .map(move |bit| (*bit).then(|| children_values_iter.next().unwrap()));

    let mut results_iter = results.iter_mut().flat_map(|results| results.iter_mut());
    (0..2_usize.pow((bitmap.results_capacity() + 1) as u32) - 1).flat_map(move |i| {
        let key = bitmap.from_index(i);
        let maybe_result = bitmap.results_bits()[i].then(|| {
            (
                key.clone(),
                ResultOrChildMut::Result(results_iter.next().unwrap()),
            )
        });
        let maybe_children = (!bitmap.is_end_node() && key.len() == bitmap.results_capacity())
            .then(|| {
                [false, true]
                    .into_iter()
                    .rev()
                    .map(move |begin_value| {
                        let mut new_key = Key::new();
                        new_key.extend(key.clone());
                        new_key.push(begin_value);
                        new_key.into_iter().collect()
                    })
                    .zip(
                        [children_iter.next().unwrap(), children_iter.next().unwrap()]
                            .into_iter()
                            .rev(),
                    )
                    .filter_map(|(key, child)| {
                        child.map(|child| (key, ResultOrChildMut::Child(child)))
                    })
                    .rev()
            })
            .into_iter()
            .flatten();
        maybe_result.into_iter().chain(maybe_children)
    })
}

enum ResultOrChild<'a, T> {
    Result(&'a T),
    Child(&'a Node<T>),
}

enum ResultOrChildMut<'a, T> {
    Result(&'a mut T),
    Child(&'a mut Node<T>),
}

impl<T: Send + Sync> Node<T> {
    fn results_and_children(&self) -> impl Iterator<Item = (Key, ResultOrChild<'_, T>)> {
        let mut children_values_iter = self.children.iter().flat_map(|children| children.iter());
        let mut children_iter = self
            .bitmap
            .children_bits()
            .iter()
            .map(move |bit| (*bit).then(|| children_values_iter.next().unwrap()));

        let mut results_iter = self.results.iter().flat_map(|results| results.iter());
        (0..2_usize.pow((self.bitmap.results_capacity() + 1) as u32) - 1).flat_map(move |i| {
            let key = self.bitmap.from_index(i);
            let maybe_result = self.bitmap.results_bits()[i].then(|| {
                (
                    key.clone(),
                    ResultOrChild::Result(results_iter.next().unwrap()),
                )
            });
            let maybe_children = (!self.bitmap.is_end_node()
                && key.len() == self.bitmap.results_capacity())
            .then(|| {
                [false, true]
                    .into_iter()
                    .rev()
                    .map(move |begin_value| {
                        let mut new_key = Key::new();
                        new_key.extend(key.clone());
                        new_key.push(begin_value);
                        new_key.into_iter().collect()
                    })
                    .zip(
                        [children_iter.next().unwrap(), children_iter.next().unwrap()]
                            .into_iter()
                            .rev(),
                    )
                    .filter_map(|(key, child)| {
                        child.map(|child| (key, ResultOrChild::Child(child)))
                    })
                    .rev()
            })
            .into_iter()
            .flatten();
            maybe_result.into_iter().chain(maybe_children)
        })
    }

    fn results_keys_and_children(&self) -> impl Iterator<Item = (Key, Option<&Node<T>>)> {
        let mut children_values_iter = self.children.iter().flat_map(|children| children.iter());
        let mut children_iter = self
            .bitmap
            .children_bits()
            .iter()
            .map(move |bit| (*bit).then(|| children_values_iter.next().unwrap()));

        (0..2_usize.pow((self.bitmap.results_capacity() + 1) as u32) - 1).flat_map(move |i| {
            let key = self.bitmap.from_index(i);
            let maybe_result = self.bitmap.results_bits()[i].then(|| (key.clone(), None));
            let maybe_children = (!self.bitmap.is_end_node()
                && key.len() == self.bitmap.results_capacity())
            .then(|| {
                [false, true]
                    .into_iter()
                    .rev()
                    .map(move |begin_value| {
                        let mut new_key = Key::new();
                        new_key.extend(key.clone());
                        new_key.push(begin_value);
                        new_key.into_iter().collect()
                    })
                    .zip(
                        [children_iter.next().unwrap(), children_iter.next().unwrap()]
                            .into_iter()
                            .rev(),
                    )
                    .filter(|(_, y)| y.is_some())
                    .rev()
            })
            .into_iter()
            .flatten();
            maybe_result.into_iter().chain(maybe_children)
        })
    }

    fn get_child(&self, key: KeyRef) -> Option<&Node<T>> {
        if self.bitmap.is_end_node() {
            return None;
        }
        let nibble: usize = key.into_iter().rev().collect::<Key>().load_le();
        self.bitmap.children_bits()[nibble].then(|| {
            let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
            &self.children.as_ref().unwrap()[vec_index]
        })
    }
    fn get_child_mut(&mut self, key: KeyRef) -> Option<&mut Node<T>> {
        if self.bitmap.is_end_node() {
            return None;
        }

        let nibble: usize = key.into_iter().rev().collect::<Key>().load_le();
        self.bitmap.children_bits()[nibble].then(|| {
            let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
            &mut self.children.as_mut().unwrap()[vec_index]
        })
    }

    fn convert_to_normal(&mut self) {
        if !self.bitmap.is_end_node() {
            return;
        }

        let results_iter = self
            .results
            .take()
            .into_iter()
            .flat_map(|results| results.into_iter());
        let results = self
            .bitmap
            .results_bits()
            .iter_ones()
            .map(|x| self.bitmap.from_index(x))
            .zip(results_iter)
            .collect::<Vec<_>>();

        self.bitmap = Default::default();
        self.bitmap.set_is_end_node(false);

        for (key, value) in results {
            self.insert(&key, value);
        }
    }
    fn get_or_insert_child(&mut self, key: KeyRef) -> &mut Node<T> {
        self.convert_to_normal();

        {
            let nibble: usize = key.into_iter().rev().collect::<Key>().load_le();
            if !self.bitmap.children_bits()[nibble] {
                self.bitmap.children_bits_mut().set(nibble, true);
                let children = self.children.get_or_insert(Default::default());
                let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
                children.insert(vec_index, Node::default());
            }
        }
        self.get_child_mut(key).unwrap()
    }

    pub fn insert(&mut self, key: KeyRef, value: T) -> Option<T> {
        if key.len() <= self.bitmap.results_capacity() {
            // capacity is suffcient, insert into local node
            let index = self.bitmap.to_index(key);

            let results = self.results.get_or_insert(Default::default());
            let vec_index = self.bitmap.results_bits()[..index].count_ones();
            if self.bitmap.results_bits()[index] {
                Some(std::mem::replace(&mut results[vec_index], value))
            } else {
                self.bitmap.results_bits_mut().set(index, true);
                results.insert(vec_index, value);
                None
            }
        } else {
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            // insert into child node
            let child = self.get_or_insert_child(key);
            child.insert(remaining, value)
        }
    }
    pub fn remove(&mut self, key: KeyRef) -> Option<T> {
        if key.len() <= self.bitmap.results_capacity() {
            let index = self.bitmap.to_index(key);
            self.bitmap.results_bits()[index].then(|| {
                self.bitmap.results_bits_mut().set(index, false);
                let results = self.results.get_or_insert(Default::default());
                let vec_index = self.bitmap.results_bits()[..index].count_ones();
                results.remove(vec_index)
            })
        } else {
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            self.get_child_mut(key)
                .and_then(|child| child.remove(remaining))
        }
    }

    fn iter_with_prefix(&self, prefix: Key) -> impl Iterator<Item = (Key, &T)> + Send + Sync + '_ {
        self.results_and_children()
            .flat_map(move |(child_or_result_key, child_or_result)| {
                let (result, from_children): (
                    _,
                    Option<Box<dyn Iterator<Item = (Key, &T)> + Send + Sync>>,
                ) = match child_or_result {
                    ResultOrChild::Result(r) => (Some((child_or_result_key, r)), None),
                    ResultOrChild::Child(child) => (
                        None,
                        Some(Box::new(child.iter_with_prefix(child_or_result_key))),
                    ),
                };
                let prefix = prefix.clone();
                result
                    .into_iter()
                    .chain(from_children.into_iter().flatten())
                    .map(move |(child_or_result_key, result)| {
                        let mut key = prefix.clone();
                        key.extend(child_or_result_key);
                        (key, result)
                    })
            })
    }
    fn iter_mut_with_prefix(
        &mut self,
        prefix: Key,
    ) -> impl Iterator<Item = (Key, &mut T)> + Send + Sync + '_ {
        results_and_children_mut(&self.bitmap, &mut self.results, &mut self.children).flat_map(
            move |(child_or_result_key, child_or_result)| {
                let (result, from_children): (
                    _,
                    Option<Box<dyn Iterator<Item = (Key, &mut T)> + Send + Sync>>,
                ) = match child_or_result {
                    ResultOrChildMut::Result(r) => (Some((child_or_result_key, r)), None),
                    ResultOrChildMut::Child(child) => (
                        None,
                        Some(Box::new(child.iter_mut_with_prefix(child_or_result_key))),
                    ),
                };
                let prefix = prefix.clone();
                result
                    .into_iter()
                    .chain(from_children.into_iter().flatten())
                    .map(move |(child_or_result_key, result)| {
                        let mut key = prefix.clone();
                        key.extend(child_or_result_key);
                        (key, result)
                    })
            },
        )
    }

    pub fn iter(&self) -> impl Iterator<Item = (Key, &T)> + '_ {
        self.iter_with_prefix(Key::new())
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Key, &mut T)> + '_ {
        self.iter_mut_with_prefix(Key::new())
    }

    pub fn values(&self) -> impl Iterator<Item = &T> + Send + Sync + '_ {
        let results_iter = self.results.iter().flat_map(|values| values.iter());
        let children_iter = self
            .children
            .iter()
            .flat_map(|children| children.iter())
            .flat_map(|child| child.values());
        let children_iter: Box<dyn Iterator<Item = &T> + Send + Sync + '_> =
            Box::new(children_iter);
        results_iter.chain(children_iter)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> + Send + Sync + '_ {
        let results_iter = self.results.iter_mut().flat_map(|values| values.iter_mut());
        let children_iter = self
            .children
            .iter_mut()
            .flat_map(|children| children.iter_mut())
            .flat_map(|child| child.values_mut());
        let children_iter: Box<dyn Iterator<Item = &mut T> + Send + Sync + '_> =
            Box::new(children_iter);
        results_iter.chain(children_iter)
    }

    fn keys_with_prefix(&self, prefix: Key) -> impl Iterator<Item = Key> + Send + Sync + '_ {
        self.results_keys_and_children()
            .flat_map(move |(child_or_result_key, child)| {
                let (result_key, children_keys): (
                    _,
                    Option<Box<dyn Iterator<Item = Key> + Send + Sync>>,
                ) = match child {
                    None => (Some(child_or_result_key), None),
                    Some(child) => (
                        None,
                        Some(Box::new(child.keys_with_prefix(child_or_result_key))),
                    ),
                };
                let prefix = prefix.clone();
                result_key
                    .into_iter()
                    .chain(children_keys.into_iter().flatten())
                    .map(move |child_or_result_key| {
                        let mut key = prefix.clone();
                        key.extend(child_or_result_key);
                        key
                    })
            })
    }

    pub fn keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.keys_with_prefix(Key::new())
    }

    pub fn exact(&self, key: KeyRef) -> Option<&T> {
        if key.len() <= self.bitmap.results_capacity() {
            let index = self.bitmap.to_index(key);
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
            let index = self.bitmap.to_index(key);
            self.bitmap.results_bits()[index].then(|| {
                let vec_index = self.bitmap.results_bits()[..index].count_ones();
                &mut self.results.as_mut().unwrap()[vec_index]
            })
        } else {
            let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
            self.get_child_mut(key)
                .and_then(|child| child.exact_mut(remaining))
        }
    }

    fn longest_match_with_prefix(&self, mut prefix: Key, mut key: KeyRef) -> Option<(Key, &T)> {
        (key.len() > self.bitmap.results_capacity())
            .then(|| {
                let mut prefix = prefix.clone();
                let (key, remaining) = key.split_at(RESULTS_BITS_END_NODE);
                prefix.extend(key);
                self.get_child(key)
                    .and_then(|child| child.longest_match_with_prefix(prefix, remaining))
            })
            .flatten()
            .or_else(|| {
                loop {
                    if let Some(result) = self.exact(key) {
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

    fn or_longer_with_prefix(
        &self,
        prefix: Key,
        mut key: Key,
    ) -> Box<dyn Iterator<Item = (Key, &T)> + Send + Sync + '_> {
        if key.len() > self.bitmap.results_capacity() {
            let mut prefix = prefix.clone();
            let remaining = key.split_off(RESULTS_BITS_END_NODE);
            prefix.extend(&key);
            if let Some(child) = self.get_child(&key) {
                Box::new(child.or_longer_with_prefix(prefix, remaining))
            } else {
                Box::new(std::iter::empty())
            }
        } else {
            Box::new(
                self.results_and_children()
                    .filter(move |(child_or_result_key, _)| child_or_result_key.starts_with(&key))
                    .flat_map(move |(child_or_result_key, child_or_result)| {
                        let (result, from_children): (
                            _,
                            Option<Box<dyn Iterator<Item = (Key, &T)> + Send + Sync>>,
                        ) = match child_or_result {
                            ResultOrChild::Result(r) => (Some((child_or_result_key, r)), None),
                            ResultOrChild::Child(child) => (
                                None,
                                Some(Box::new(child.iter_with_prefix(child_or_result_key))),
                            ),
                        };
                        let prefix = prefix.clone();
                        result
                            .into_iter()
                            .chain(from_children.into_iter().flatten())
                            .map(move |(child_or_result_key, result)| {
                                let mut key = prefix.clone();
                                key.extend(child_or_result_key);
                                (key, result)
                            })
                    }),
            )
        }
    }
    pub fn or_longer(&self, key: Key) -> impl Iterator<Item = (Key, &T)> + '_ {
        self.or_longer_with_prefix(Key::new(), key)
    }

    fn matches_with_prefix(
        &self,
        prefix: Key,
        mut key: Key,
    ) -> impl Iterator<Item = (Key, &T)> + Send + Sync + '_ {
        self.results_and_children()
            .filter({
                let key = key.clone();
                move |(child_or_result_key, _)| key.starts_with(child_or_result_key)
            })
            .flat_map(move |(child_or_result_key, child_or_result)| {
                let (result, from_children): (
                    _,
                    Option<Box<dyn Iterator<Item = (Key, &T)> + Send + Sync>>,
                ) = match child_or_result {
                    ResultOrChild::Result(r) => (Some((child_or_result_key, r)), None),
                    ResultOrChild::Child(child) => {
                        let remaining = key.split_off(RESULTS_BITS_END_NODE);
                        (
                            None,
                            Some(Box::new(
                                child.matches_with_prefix(child_or_result_key, remaining),
                            )),
                        )
                    }
                };
                let prefix = prefix.clone();
                result
                    .into_iter()
                    .chain(from_children.into_iter().flatten())
                    .map(move |(child_or_result_key, result)| {
                        let mut key = prefix.clone();
                        key.extend(child_or_result_key);
                        (key, result)
                    })
            })
    }
    pub fn matches(&self, key: Key) -> impl Iterator<Item = (Key, &T)> + '_ {
        self.matches_with_prefix(Key::new(), key)
    }
}
