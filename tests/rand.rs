use rand_xoshiro::Xoshiro256StarStar;
use rstest::rstest;
use rstest_reuse::{self, *};
use rand::prelude::*;
use bitvec::prelude::*;
use nibbletree::{Key, Node};

fn random_tree(len: usize, max_key_len: usize, seed: u8) -> (Vec<(Key, u64)>, Node<Key, u64>) {
    let mut rng = Xoshiro256StarStar::from_seed([seed; 32]);
    let mut data = (0..len)
        .map(|_| {
            let key_len = rng.gen_range(0..max_key_len);
            let mut key = bitvec![0u8; key_len];
            rng.fill(key.as_raw_mut_slice());
            (key, rng.gen())
        })
    .collect::<Vec<_>>();
    data.sort();
    data.dedup_by(|(a, _), (b, _)| a == b);
    data.shuffle(&mut rng);

    let mut tree = Node::default();
    for (key, value) in &data {
        tree.insert(key, *value);
    }

    (data, tree)
}

#[template]
#[rstest]
fn random_tree_template(
    #[values(1, 10, 100, 1000)]
    len: usize,
    #[values(4, 32, 128)]
    max_key_len: usize,
    #[values(1, 2, 3)]
    seed: u8
) {}

#[apply(random_tree_template)]
fn iter(len: usize, max_key_len: usize, seed: u8) {
    let (mut data, tree) = random_tree(len, max_key_len, seed);

    data.sort();
    let mut out = tree.iter().map(|(k, v)| (k, *v)).collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}

#[apply(random_tree_template)]
fn remove(len: usize, max_key_len: usize, seed: u8) {
    let (mut data, mut tree) = random_tree(len, max_key_len, seed);

    let to_be_removed = data.split_off(data.len() / 2);
    let removed = to_be_removed.iter().map(|(key, _)| (key.clone(), tree.remove(&key).unwrap())).collect::<Vec<_>>();
    assert_eq!(to_be_removed, removed);

    data.sort();
    let mut out = tree.iter().map(|(k, v)| (k, *v)).collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}
