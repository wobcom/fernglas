use std::ops::{Index, IndexMut};
use std::fmt::{Debug, Formatter};
use bitvec::prelude::*;

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
    fn is_end_node(&self) -> bool {
        self.bitmap.view_bits::<Msb0>()[0]
    }
    fn set_is_end_node(&mut self, is_end_node: bool) {
        self.bitmap.view_bits_mut::<Msb0>().set(0, is_end_node);
    }
    fn children_start_at(&self) -> usize {
        if self.is_end_node() { 32 } else { 16 }
    }
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

    fn results_keys_with_prefix(&self, prefix: Vec<u8>) -> impl Iterator<Item = (Vec<u8>, u32)> + '_ {
        self.results_bits()
            .iter_ones()
            .map(from_index)
            .map(move |(nibble, result_prefix_len)| {
                let mut key = prefix.clone();
                key.push(nibble);
                let prefix_len = prefix.len() as u32 * 4 + result_prefix_len;
                (key, prefix_len)
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

    fn children(&self) -> impl Iterator<Item = (u8, &Node<T>)> {
        let children_iter = self.children.iter().flat_map(|children| children.iter());
        self.bitmap.children_bits().iter_ones().map(|x| x as u8).zip(children_iter)
    }
    fn children_mut(&mut self) -> impl Iterator<Item = (u8, &mut Node<T>)> {
        let children_iter = self.children.iter_mut().flat_map(|children| children.iter_mut());
        self.bitmap.children_bits().iter_ones().map(|x| x as u8).zip(children_iter)
    }

    fn results(&self) -> impl Iterator<Item = ((u8, u32), &T)> {
        let results_iter = self.results.iter().flat_map(|results| results.iter());
        self.bitmap.results_bits().iter_ones().map(from_index).zip(results_iter)
    }
    fn results_mut(&mut self) -> impl Iterator<Item = ((u8, u32), &mut T)> {
        let results_iter = self.results.iter_mut().flat_map(|results| results.iter_mut());
        self.bitmap.results_bits().iter_ones().map(from_index).zip(results_iter)
    }

    fn get_child(&self, nibble: u8) -> Option<&Node<T>> {
        let nibble = nibble as usize;
        self.bitmap.children_bits()[nibble].then(|| {
            let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
            &self.children.as_ref().unwrap()[vec_index]
        })
    }
    fn get_child_mut(&mut self, nibble: u8) -> Option<&mut Node<T>> {
        let nibble = nibble as usize;
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

        for ((nibble, prefix_len), value) in results {
            self.insert(&[nibble], prefix_len, value);
        }
    }
    fn get_or_insert_child(&mut self, nibble: u8) -> &mut Node<T> {
        self.convert_to_normal();

        {
            let nibble = nibble as usize;
            if !self.bitmap.children_bits()[nibble] {
                self.bitmap.children_bits_mut().set(nibble, true);
                let mut children = self.children.get_or_insert(Default::default());
                let vec_index = self.bitmap.children_bits()[..nibble].count_ones();
                children.insert(vec_index, Node::new());
            }
        }
        self.get_child_mut(nibble).unwrap()
    }

    pub fn insert(&mut self, nibbles: &[u8], prefix_len: u32, value: T) {
        if prefix_len as usize <= self.bitmap.results_capacity() {
            // capacity is suffcient, insert into local node
            let nibble = *nibbles.get(0).unwrap_or(&0);
            let index = to_index(nibble, prefix_len);
            self.bitmap.results_bits_mut().set(index, true);
            let mut results = self.results.get_or_insert(Default::default());
            let vec_index = self.bitmap.results_bits()[..index].count_ones();
            results.insert(vec_index, value);
        } else {
            // insert into child node
            let child = self.get_or_insert_child(nibbles[0]);
            child.insert(&nibbles[1..], prefix_len - 4, value);
        }
    }

    fn iter_with_prefix(&self, prefix: Vec<u8>) -> impl Iterator<Item = ((Vec<u8>, u32), &T)> + '_ {
        let results_iter = {
            let prefix = prefix.clone();
            self.results().map(move |((nibble, result_prefix_len), val)| {
                let mut key = prefix.clone();
                key.push(nibble);
                let prefix_len = prefix.len() as u32 * 4 + result_prefix_len;
                ((key, prefix_len), val)
            })
        };
        let children_iter = self.children()
            .flat_map(move |(nibble, child)| {
                let mut key = prefix.clone();
                key.push(nibble);
                child.iter_with_prefix(key)
            });
        let children_iter: Box<dyn Iterator<Item = ((Vec<u8>, u32), &T)> + '_>  = Box::new(children_iter);
        results_iter.chain(children_iter)
    }

    pub fn iter(&self) -> impl Iterator<Item = ((Vec<u8>, u32), &T)> + '_ {
        self.iter_with_prefix(vec![])
    }

    pub fn values(&self) -> impl Iterator<Item = &T> + '_ {
        let results_iter = self.results.iter().flat_map(|values| values.iter());
        let children_iter = self.children.iter()
            .flat_map(|children| children.iter())
            .flat_map(|child| child.values());
        let children_iter: Box<dyn Iterator<Item = &T> + '_> = Box::new(children_iter);
        results_iter.chain(children_iter)
    }

    fn keys_with_prefix<'a>(&'a self, prefix: Vec<u8>) -> impl Iterator<Item = (Vec<u8>, u32)> + '_ {
        let results_keys_iter = self.bitmap.results_keys_with_prefix(prefix.clone());
        let children_keys_iter = self.children()
            .flat_map(move |(nibble, child)| {
                let mut key = prefix.clone();
                key.push(nibble);
                println!("child {:?}", key);
                child.keys_with_prefix(key)
            });
        let children_keys_iter: Box<dyn Iterator<Item = (Vec<u8>, u32)> + '_> = Box::new(children_keys_iter);
        results_keys_iter.chain(children_keys_iter)
    }

    pub fn keys(&self) -> impl Iterator<Item = (Vec<u8>, u32)> + '_ {
        self.keys_with_prefix(vec![])
    }
}

fn to_index(nibble: u8, prefix_len: u32) -> usize {
    let leading_one = 2usize.pow(prefix_len as u32);
    let net_bits = nibble >> (4 - prefix_len as usize);
    (leading_one + net_bits as usize) - 1
}

fn from_index(mut index: usize) -> (u8, u32) {
    index += 1;
    let prefix_len = (std::mem::size_of::<usize>() as u32 * 8) - index.leading_zeros() - 1;
    index -= 2usize.pow(prefix_len);
    index <<= 4 - prefix_len as u32;
    (index as u8, prefix_len)
}
