use rand_xoshiro::Xoshiro256StarStar;
use rstest::rstest;
use rstest_reuse::{self, *};
use rand::prelude::*;
use bitvec::prelude::*;
use nibbletree::{Key, Node};

fn rand_key(max_key_len: usize, rng: &mut impl Rng) -> Key {
    let key_len = rng.gen_range(0..max_key_len);
    let mut key = bitvec![0u8; key_len];
    rng.fill(key.as_raw_mut_slice());
    key
}

fn random_tree(len: usize, max_key_len: usize, seed: u8) -> (Vec<(Key, u64)>, Node<Key, u64>, Vec<Key>) {
    let mut rng = Xoshiro256StarStar::from_seed([seed; 32]);
    let mut data = (0..len)
        .map(|_| (rand_key(max_key_len, &mut rng), rng.gen()))
        .collect::<Vec<_>>();
    data.sort();
    data.dedup_by_key(|(k, _)| k.clone());
    data.shuffle(&mut rng);

    let mut tree = Node::default();
    for (key, value) in &data {
        tree.insert(key, *value);
    }

    let test_keys = (0..100)
        .map(|_| rand_key(max_key_len, &mut rng))
        .collect::<Vec<_>>();

    (data, tree, test_keys)
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
    let (mut data, tree, _) = random_tree(len, max_key_len, seed);

    data.sort();
    let mut out = tree.iter().map(|(k, v)| (k, *v)).collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}

#[apply(random_tree_template)]
fn iter_mut(len: usize, max_key_len: usize, seed: u8) {
    let (mut data, mut tree, _) = random_tree(len, max_key_len, seed);

    data.sort();
    let mut out = tree.iter_mut().map(|(k, v)| (k, *v)).collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}

#[apply(random_tree_template)]
fn keys(len: usize, max_key_len: usize, seed: u8) {
    let (data, tree, _) = random_tree(len, max_key_len, seed);

    let mut data = data.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
    data.sort();
    let mut out = tree.keys().collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}

#[apply(random_tree_template)]
fn values(len: usize, max_key_len: usize, seed: u8) {
    let (data, tree, _) = random_tree(len, max_key_len, seed);

    let mut data = data.iter().map(|(_, v)| v).collect::<Vec<_>>();
    data.sort();
    let mut out = tree.values().collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}

#[apply(random_tree_template)]
fn values_mut(len: usize, max_key_len: usize, seed: u8) {
    let (data, mut tree, _) = random_tree(len, max_key_len, seed);

    let mut data = data.iter().map(|(_, v)| v).collect::<Vec<_>>();
    data.sort();
    let mut out = tree.values_mut().collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}

#[apply(random_tree_template)]
fn remove(len: usize, max_key_len: usize, seed: u8) {
    let (mut data, mut tree, _) = random_tree(len, max_key_len, seed);

    let to_be_removed = data.split_off(data.len() / 2);
    let removed = to_be_removed.iter().map(|(key, _)| (key.clone(), tree.remove(&key).unwrap())).collect::<Vec<_>>();
    assert_eq!(to_be_removed, removed);

    data.sort();
    let mut out = tree.iter().map(|(k, v)| (k, *v)).collect::<Vec<_>>();
    out.sort();
    assert_eq!(data, out);
}

#[apply(random_tree_template)]
fn exact(len: usize, max_key_len: usize, seed: u8) {
    let (data, tree, test_keys) = random_tree(len, max_key_len, seed);

    for key in test_keys {
        let should_match = data
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v);
        let is_match = tree.exact(&key);
        assert_eq!(should_match, is_match);
    }
}

#[apply(random_tree_template)]
fn longest_match(len: usize, max_key_len: usize, seed: u8) {
    let (data, tree, test_keys) = random_tree(len, max_key_len, seed);

    for key in test_keys {
        let should_match = data
            .iter()
            .filter(|(k, _)| key.starts_with(&k))
            .max_by_key(|(k, _)| k.len())
            .map(|(k, v)| (k.clone(), v));
        let is_match = tree.longest_match(&key);
        assert_eq!(should_match, is_match);
    }
}

#[apply(random_tree_template)]
fn or_longer(len: usize, max_key_len: usize, seed: u8) {
    let (data, tree, test_keys) = random_tree(len, max_key_len, seed);

    for key in test_keys {
        let mut should_match = data
            .iter()
            .filter(|(k, _)| k.starts_with(&key))
            .map(|(k, v)| (k.clone(), v))
            .collect::<Vec<_>>();
        let mut is_match = tree
            .or_longer(&key)
            .collect::<Vec<_>>();
        should_match.sort();
        is_match.sort();
        assert_eq!(should_match, is_match);
    }
}

#[apply(random_tree_template)]
fn matches(len: usize, max_key_len: usize, seed: u8) {
    let (data, tree, test_keys) = random_tree(len, max_key_len, seed);

    for key in test_keys {
        let mut should_match = data
            .iter()
            .filter(|(k, _)| key.starts_with(k))
            .map(|(k, v)| (k.clone(), v))
            .collect::<Vec<_>>();
        let mut is_match = tree
            .matches(&key)
            .collect::<Vec<_>>();
        should_match.sort();
        is_match.sort();
        assert_eq!(should_match, is_match);
    }
}
