use crate::store::PathId;
use crate::table_impl::*;
use ipnet::IpNet;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Notify;

struct TableStreamState<T> {
    buf: Vec<(IpNet, PathId, Action<T>)>,
    // Overflow table; used when the buffer is full
    // IMPORTANT: only stores the latest Action for each prefix,
    // so it will never get larger than a full table
    overflow: InMemoryTableState<Action<T>>,
}
impl<T> Default for TableStreamState<T> {
    fn default() -> Self {
        Self {
            buf: Vec::with_capacity(128),
            overflow: Default::default(),
        }
    }
}
impl<T: Send + Sync + Clone> Extend<(IpNet, PathId, Action<T>)> for TableStreamState<T> {
    fn extend<I>(&mut self, i: I)
    where
        I: IntoIterator<Item = (IpNet, PathId, Action<T>)>,
    {
        let mut iter = i.into_iter();
        while self.buf.len() < 128 {
            let Some(next) = iter.next() else {
                break;
            };
            self.buf.push(next);
        }
        self.overflow.extend(iter)
    }
}
impl<T: Send + Sync + Clone> FromIterator<(IpNet, PathId, Action<T>)> for TableStreamState<T> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (IpNet, PathId, Action<T>)>,
    {
        let mut this: Self = Default::default();
        this.extend(iter);
        this
    }
}

/// Provides a flow-controlled Stream monitoring all actions on a table, suitable for use with slow readers
pub fn table_stream<C, T>(
    table: &InMemoryTable<T>,
) -> impl futures_util::Stream<Item = (IpNet, PathId, Action<C>)>
where
    C: Clone + Send + Sync + 'static + PartialEq + std::fmt::Debug,
    T: Compressable<Compressed = C> + Clone + 'static,
{
    // Keeps track of the last state sent to the peer
    let mut rib_out: InMemoryTableState<C> = Default::default();

    let state: TableStreamState<C> =
        // Copy the initial contents of the table
        table.state.lock().unwrap().table.iter()
        .flat_map(|(net, v)| v.iter().map(move |(num, attrs)| (net, *num, Action::Update(attrs.clone()))))
        .collect();
    let state = Arc::new(Mutex::new(state));
    // Used to wake up the reader task when new routes have been inserted into the state
    let notify = Arc::new(Notify::new());
    // Mark as dirty, since we added initial contents
    notify.notify_one();

    let subscriber: Arc<Subscriber<_>> = {
        let state = state.clone();
        let notify = notify.clone();
        Arc::new(move |net, num, action| {
            state.lock().unwrap().extend(Some((net, num, action)));
            notify.notify_one();
        })
    };
    table.subscribe(Arc::downgrade(&subscriber));
    async_stream::stream! {
        let mut local_state: TableStreamState<C> = Default::default();
        #[allow(unused)]
        let subscriber = subscriber;
        loop {
            notify.notified().await;
            std::mem::swap(&mut local_state, &mut *state.lock().unwrap());

            if local_state.buf.len() >= 128 {
                // We merge the buf into the overflow table, but so that items from the overflow
                // table are preferred (since they arrived later)
                for (net, num, action) in local_state.buf.drain(..).rev() {
                    let overflow_entry = local_state.overflow.table.exact(&net).and_then(|entry| {
                        match entry.binary_search_by_key(&num, |(k, _)| *k) {
                            Ok(index) => Some(entry[index].1.clone()),
                            Err(_) => None,
                        }
                    });
                    if overflow_entry.is_none() {
                        local_state.overflow.update_route(num, net, action);
                    }
                }

                for (net, v) in local_state.overflow.table.iter() {
                    for (num, action) in v {
                        // filter out routes that have flapped back to the state we last emitted to the stream
                        let rib_out_entry = rib_out.table.exact(&net).and_then(|entry| {
                            match entry.binary_search_by_key(&num, |(k, _)| k) {
                                Ok(index) => Some(entry[index].1.clone()),
                                Err(_) => None,
                            }
                        });
                        let actual_action = match (action, rib_out_entry) {
                            (Action::Update(attrs), None) => Some(Action::Update(attrs.clone())),
                            (Action::Update(attrs), Some(existing_route)) if *attrs != existing_route => Some(Action::Update(attrs.clone())),
                            (Action::Withdraw, Some(_)) => Some(Action::Withdraw),
                            _ => None,
                        };
                        rib_out.extend(actual_action.map(|action| (net, *num, action)));
                        yield (net, *num, action.clone());
                    }
                }

                drop(std::mem::take(&mut local_state.overflow));
            } else {
                for i in local_state.buf.drain(..) {
                    rib_out.extend(Some(i.clone()));
                    yield i;
                }
            }
        }

        // Keeps the subscription of table events alive as long as the Stream exists
        #[allow(unreachable_code)]
        drop(subscriber);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressed_attrs::decompress_route_attrs;
    use crate::store::RouteAttrs;
    use futures_util::pin_mut;
    use futures_util::StreamExt;
    use rstest::rstest;
    use std::time::Duration;

    #[rstest]
    #[tokio::test]
    async fn test_table_stream(
        #[values(false, true)] drain_in_between: bool,
        #[values(1, 64, 127, 128, 129, 130, 255)] j1: u8,
        #[values(1, 64, 127, 128, 129, 130, 255)] j2: u8,
        #[values(1, 64, 127, 128, 129, 130, 255)] j3: u8,
    ) {
        let caches: Arc<Mutex<_>> = Default::default();
        let prefix = "0.0.0.0/0".parse().unwrap();
        let table_in: InMemoryTable = InMemoryTable::new(caches.clone());
        let table_stream = table_stream(&table_in);
        pin_mut!(table_stream);
        let mut sent_count: usize = 0;
        for j in [&j1, &j2, &j3] {
            for i in 0..*j {
                table_in.update_route(
                    0,
                    prefix,
                    RouteAttrs {
                        nexthop: Some([10u8, 0u8, 0u8, i].into()),
                        ..Default::default()
                    },
                );
            }
            let mut table_out: InMemoryTableState = Default::default();
            sent_count += *j as usize;
            if std::ptr::eq(j, &j3) || drain_in_between {
                let sent_count = std::mem::take(&mut sent_count);
                let expected_count = if sent_count < 128 {
                    // The messages fit into the buffer and are delivered as-is
                    sent_count
                } else {
                    // The buffer has overflowed and all routes have been compressed into one
                    1
                };
                for _ in 0..expected_count {
                    let r = tokio::time::timeout(Duration::from_secs(1), table_stream.next())
                        .await
                        .unwrap()
                        .unwrap();
                    table_out.extend(Some(r));
                }

                // The last value received is the last value inserted into table_in
                assert_eq!(
                    table_out
                        .table
                        .exact(&prefix)
                        .into_iter()
                        .flat_map(|r| r.iter())
                        .map(|(_num, r)| decompress_route_attrs(r).nexthop)
                        .collect::<Vec<_>>(),
                    vec![Some([10u8, 0u8, 0u8, j - 1].into())]
                );
            }
        }
    }
}
