use crate::store::PathId;
use crate::table_impl::*;
use ipnet::IpNet;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::Notify;

/// Provides a flow-controlled Stream monitoring all actions on a table, suitable for use with slow readers
pub fn table_stream<C, T>(
    table: &InMemoryTable<T>,
) -> impl futures_util::Stream<Item = (IpNet, PathId, Action<C>)>
where
    C: Clone + Send + Sync + 'static + PartialEq,
    T: Compressable<Compressed = C> + Clone + 'static,
{
    // The subscriber primarily sends table events here
    let (tx, mut rx) = mpsc::channel(128);

    // Keeps track of the last state sent to the peer
    let mut rib_out: InMemoryTableState<C> = Default::default();

    // Overflow table; used when the channel is full
    // IMPORTANT: only stores the latest Action for each prefix,
    // so it will never get larger than a full table
    let overflow_table: InMemoryTableState<Action<C>> =
        // Copy the initial contents of the table
        table.state.lock().unwrap().table.iter()
        .flat_map(|(net, v)| v.iter().map(move |(num, attrs)| (net, *num, Action::Update(attrs.clone()))))
        .collect();
    let overflow_table = Arc::new(Mutex::new(overflow_table));
    // Used to wake up the reader task when new routes have been inserted into overflow_table
    let overflow_table_notify = Arc::new(Notify::new());
    // Mark as dirty, since we added initial contents
    overflow_table_notify.notify_one();

    let subscriber: Arc<Subscriber<_>> = {
        let overflow_table = overflow_table.clone();
        let overflow_table_notify = overflow_table_notify.clone();
        Arc::new(move |net, num, action| {
            if let Ok(permit) = tx.try_reserve() {
                permit.send((net, num, action));
            } else {
                {
                    let mut overflow_table = overflow_table.lock().unwrap();
                    overflow_table.update_route(num, net, action);
                }
                overflow_table_notify.notify_one();
            }
        })
    };
    table.subscribe(Arc::downgrade(&subscriber));
    async_stream::stream! {
        // Counting how many routes have been processed in a pass-through phase
        let mut passthrough_processed = 0;
        let mut overflow_processed = 0;
        loop {
            tokio::select! {
                _ = overflow_table_notify.notified() => {
                    // the channel overflowed and the subscriber placed the new routes in the
                    // overflow table
                    if overflow_processed == 0 {
                        let passthrough_processed = std::mem::take(&mut passthrough_processed);
                        if passthrough_processed != 0 {
                            log::debug!("overflow, processed {} in pass-through phase", passthrough_processed);
                        }
                    }

                    let mut processed = 0;
                    loop {
                        let overflow_table = std::mem::take(&mut *overflow_table.lock().unwrap());

                        for (net, entry) in overflow_table.table.iter() {
                            for (num, action) in entry.iter() {
                                let rib_out_entry = rib_out.table.exact(&net).and_then(|entry| {
                                    match entry.binary_search_by_key(&num, |(k, _)| k) {
                                        Ok(index) => Some(entry[index].1.clone()),
                                        Err(_) => None,
                                    }
                                });
                                let actual_action = match (action, rib_out_entry) {
                                    (Action::Update(attrs), None) => Some(Action::Update(attrs)),
                                    (Action::Update(attrs), Some(existing_route)) if *attrs != existing_route => Some(Action::Update(attrs)),
                                    (Action::Withdraw, Some(_)) => Some(Action::Withdraw),
                                    _ => None,
                                };
                                let Some(actual_action) = actual_action else { continue };

                                processed += 1;
                                match actual_action {
                                    Action::Update(attrs) => {
                                        rib_out.update_route(*num, net, attrs.clone());
                                    }
                                    Action::Withdraw => {
                                        rib_out.withdraw_route(*num, net);
                                    }
                                }
                                yield (net, *num, action.clone());
                            }
                        }

                        if processed == 0 {
                            break;
                        }
                        overflow_processed += std::mem::take(&mut processed);
                    }

                }
                entry = rx.recv() => {
                    if passthrough_processed == 0 {
                        let overflow_processed = std::mem::take(&mut overflow_processed);
                        if overflow_processed != 0 {
                            log::debug!("caught up, processed {} in overflow phase", overflow_processed);
                        }
                    }
                    let Some((net, num, action)) = entry else { break; };
                    passthrough_processed += 1;
                    match &action {
                        Action::Update(attrs) => rib_out.update_route(num, net, attrs.clone()),
                        Action::Withdraw => rib_out.withdraw_route(num, net),
                    }
                    yield (net, num, action);
                }
            }
        }
        // Keeps the subscription of table events alive as long as the Stream exists
        drop(subscriber);
    }
}
