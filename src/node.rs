use std::ops::{Index, IndexMut};
use std::fmt::{Debug, Formatter};
use bitvec::prelude::*;

pub type Key = BitVec<u8, Msb0>;

#[derive(Debug)]
pub struct Node<T> {
    results: Option<Box<Vec<T>>>,
    children: Option<Box<Vec<Node<T>>>>,
    bitmap: Bitmap,
}

struct Bitmap {
    bitmap: u32,
}

impl Debug for Bitmap {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let field2_name = if self.is_end_node() { "internal2" } else { "external" };
        f.debug_struct("Bitmap")
            .field("is_end_node", &self.is_end_node())
            .field("internal", &format!("{}", &self.bitmap.view_bits::<Msb0>()[..16]))
            .field(field2_name, &format!("{}", &self.bitmap.view_bits::<Msb0>()[16..]))
            .finish()
    }
}

impl Bitmap {
    #[inline]
    fn is_end_node(&self) -> bool {
        self.bitmap.view_bits::<Msb0>()[0]
    }
    fn set_is_end_node(&mut self, is_end_node: bool) {
        self.bitmap.view_bits_mut::<Msb0>().set(0, is_end_node);
    }
    #[inline]
    fn children_start_at(&self) -> usize {
        if self.is_end_node() { 32 } else { 16 }
    }
    #[inline]
    fn results_capacity(&self) -> usize {
        if self.is_end_node() { 4 } else { 3 }
    }

    fn children_bits_mut(&mut self) -> &mut BitSlice<u32, Msb0> {
        let start = self.children_start_at();
        self.bitmap.view_bits_mut::<Msb0>().index_mut(start..)
    }
    fn children_bits(&self) -> &BitSlice<u32, Msb0> {
        let start = self.children_start_at();
        self.bitmap.view_bits::<Msb0>().index(start..)
    }

    fn results_bits_mut(&mut self) -> &mut BitSlice<u32, Msb0> {
        let end = self.children_start_at();
        self.bitmap.view_bits_mut::<Msb0>().index_mut(1..end)
    }
    fn results_bits(&self) -> &BitSlice<u32, Msb0> {
        let end = self.children_start_at();
        self.bitmap.view_bits::<Msb0>().index(1..end)
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

impl<T: Debug> Default for Node<T> {
    fn default() -> Self { Node::new() }
}

impl<T: Debug> Node<T> {
    pub fn new() -> Node<T> {
        let mut bitmap = Bitmap { bitmap: 0 };
        bitmap.set_is_end_node(true);
        Node {
            results: None,
            children: None,
            bitmap,
        }
    }

    fn children(&self) -> impl Iterator<Item = (Key, &Node<T>)> {
        let children_iter = self.children.iter().flat_map(|children| children.iter());
        self.bitmap.children_bits().iter_ones().map(|x| x.view_bits::<Msb0>().iter().rev().take(4).rev().collect()).zip(children_iter)
    }
    fn children_mut(&mut self) -> impl Iterator<Item = (Key, &mut Node<T>)> {
        let children_iter = self.children.iter_mut().flat_map(|children| children.iter_mut());
        self.bitmap.children_bits().iter_ones().map(|x| x.view_bits::<Msb0>().iter().rev().take(4).rev().collect()).zip(children_iter)
    }

    fn results(&self) -> impl Iterator<Item = (Key, &T)> {
        let results_iter = self.results.iter().flat_map(|results| results.iter());
        self.bitmap.results_bits().iter_ones().map(from_index).zip(results_iter)
    }
    fn results_mut(&mut self) -> impl Iterator<Item = (Key, &mut T)> {
        let results_iter = self.results.iter_mut().flat_map(|results| results.iter_mut());
        self.bitmap.results_bits().iter_ones().map(from_index).zip(results_iter)
    }

    fn get_child(&self, key: Key) -> Option<&Node<T>> {
        let nibble: usize = key.load_be();
        self.bitmap.children_bits()[nibble].then(|| {
            let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
            &self.children.as_ref().unwrap()[vec_index]
        })
    }
    fn get_child_mut(&mut self, key: Key) -> Option<&mut Node<T>> {
        let nibble: usize = key.load_be();
        self.bitmap.children_bits()[nibble].then(|| {
            let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
            &mut self.children.as_mut().unwrap()[vec_index]
        })
    }

    fn convert_to_normal(&mut self) {
        if !self.bitmap.is_end_node() { return; }

        let results_iter = self.results.take().into_iter().flat_map(|results| results.into_iter());
        let results = self.bitmap.results_bits().iter_ones().map(from_index).zip(results_iter).collect::<Vec<_>>();

        self.bitmap = Bitmap { bitmap: 0 };
        self.bitmap.set_is_end_node(false);

        for (key, value) in results {
            self.insert(key, value);
        }
    }
    fn get_or_insert_child(&mut self, key: Key) -> &mut Node<T> {
        self.convert_to_normal();

        {
            let nibble: usize = key.load_be();
            if !self.bitmap.children_bits()[nibble] {
                self.bitmap.children_bits_mut().set(nibble, true);
                let children = self.children.get_or_insert(Default::default());
                let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
                children.insert(vec_index, Node::new());
            }
        }
        self.get_child_mut(key).unwrap()
    }

    pub fn insert(&mut self, mut key: Key, value: T) {
        if key.len() <= self.bitmap.results_capacity() {
            // capacity is suffcient, insert into local node
            let index = to_index(key);
            self.bitmap.results_bits_mut().set(index, true);
            let results = self.results.get_or_insert(Default::default());
            let vec_index = self.bitmap.results_bits()[..index].count_ones();
            results.insert(vec_index, value);
        } else {
            let remaining = key.split_off(4);
            // insert into child node
            let child = self.get_or_insert_child(key);
            child.insert(remaining, value);
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

    pub fn iter(&self) -> impl Iterator<Item = (Key, &T)> + '_ {
        self.iter_with_prefix(Key::new())
    }

    pub fn values(&self) -> impl Iterator<Item = &T> + '_ {
        let results_iter = self.results.iter().flat_map(|values| values.iter());
        let children_iter = self.children.iter()
            .flat_map(|children| children.iter())
            .flat_map(|child| child.values());
        let children_iter: Box<dyn Iterator<Item = &T> + '_> = Box::new(children_iter);
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
}

fn to_index(key: Key) -> usize {
    let leading_one = 2usize.pow(key.len() as u32);
    let net_bits: usize = if key.is_empty() { 0 } else { key.load_be() };
    (leading_one + net_bits) - 1
}

fn from_index(mut index: usize) -> Key {
    index += 1;
    let prefix_len = (std::mem::size_of::<usize>() as u32 * 8) - index.leading_zeros() - 1;
    let mut key = Key::new();
    key.extend(index.view_bits::<Msb0>().iter().rev().take(prefix_len as usize).rev());
    key
}
