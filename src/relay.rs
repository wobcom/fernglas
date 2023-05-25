use serde::Deserialize;
use ipnet::IpNet;
use std::cmp::Ordering;
use std::sync::Arc;

use crate::store::*;
use crate::store_impl::*;
use crate::compressed_attrs::*;
use crate::table_impl::*;

#[derive(Debug, Deserialize)]
pub struct RelayConfig {
    table: TableSelector,
}

pub enum Action {
    Update(IpNet, u32, Arc<CompressedRouteAttrs>),
    Withdraw(IpNet, u32),
}

fn diff<S, T: Clone, U: Ord, F: Fn(&T) -> U, F1: Fn(&mut S, &T), F2: Fn(&mut S, &T), F3: Fn(&mut S, &T, &T)>(state: &mut S, table: &Vec<T>, rib_out: &Vec<T>, key_fn: F, update_fn: F1, withdraw_fn: F2, keep_fn: F3) {
    let mut table_iter = table.iter().peekable();
    let mut rib_out_iter = rib_out.iter().peekable();

    loop {
        match (table_iter.peek(), rib_out_iter.peek()) {
            (Some(a), Some(b)) => {
                match key_fn(a).cmp(&key_fn(b)) {
                    Ordering::Less => update_fn(state, table_iter.next().unwrap()),
                    Ordering::Greater => withdraw_fn(state, rib_out_iter.next().unwrap()),
                    Ordering::Equal => keep_fn(state, table_iter.next().unwrap(), rib_out_iter.next().unwrap()),
                }
            },
            (Some(_), None) => update_fn(state, table_iter.next().unwrap()),
            (None, Some(_)) => withdraw_fn(state, rib_out_iter.next().unwrap()),
            (None, None) => break,
        }
    }
}

async fn run_(cfg: RelayConfig, store: InMemoryStore) -> anyhow::Result<()> {
    let rib_out = InMemoryTable::new(store.caches.clone());
    loop {
        {
            let mut changes = vec![];
            {
                let table = store.get_table(cfg.table.clone());
                let table = table.state.lock().unwrap();
                let rib_out = rib_out.state.lock().unwrap();
                let apply_nums = |s: &mut Vec<Action>, net: &IpNet, x: &Vec<(u32, Arc<CompressedRouteAttrs>)>, y: &Vec<(u32, Arc<CompressedRouteAttrs>)>| {
                    diff(s, x, y, |x| x.0, |s, x| {
                        s.push(Action::Update(*net, x.0, x.1.clone()));
                    }, |s, x| {
                        s.push(Action::Withdraw(*net, x.0));
                    }, |s, x, y| {
                        if x != y {
                            s.push(Action::Update(*net, x.0, x.1.clone()));
                        }
                    });
                };

                diff(
                    &mut changes,
                    &table.vec,
                    &rib_out.vec,
                    |x| *x,
                    |s, x| {
                        apply_nums(s, x, table.table.exact(&x).unwrap(), &vec![]);
                    },
                    |s, x| {
                        apply_nums(s, x, &vec![], rib_out.table.exact(&x).unwrap());
                    },
                    |s, x, y| {
                        apply_nums(s, x, table.table.exact(&x).unwrap(), rib_out.table.exact(&y).unwrap());
                    }
                );
            }

            let changes_empty = changes.is_empty();
            println!("{:?} changes", changes.len());
            for change in changes {
                match change {
                    Action::Update(net, num, attrs) => rib_out.update_route_compressed(num, net, attrs).await,
                    Action::Withdraw(net, num) => rib_out.withdraw_route(num, net).await,
                }
            }
            println!("{:?} rib out", rib_out.state.lock().unwrap().vec.len());
            if changes_empty {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

}

pub async fn run(cfg: RelayConfig, store: InMemoryStore, mut shutdown: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()> {
    tokio::select! {
        res = run_(cfg, store) => return res,
        _ = shutdown.changed() => return Ok(()),
    }
}
