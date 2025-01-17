use crate::iter_diff::*;
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
    let (tx, mut rx) = mpsc::channel(16);

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
        // Counting how many routes have been processed in an overflow (Err) or pass-through (Ok)
        // phase. We start out in an Err phase, because the initial table dump is placed in the
        // overflow table.
        let mut num_processed = Err(0);
        loop {
            tokio::select! {
                _ = overflow_table_notify.notified() => {
                    // the channel overflowed and the subscriber placed the new routes in the
                    // overflow table
                    let overflow_table = std::mem::take(&mut *overflow_table.lock().unwrap());
                    if let Ok(passthrough_processed) = &num_processed {
                        // We are entering an overflow phase
                        log::debug!("overflow, processed in pass-through phase: {}", passthrough_processed);
                        num_processed = Err(0);
                    }
                    let overflow_processed = num_processed.as_mut().unwrap_err();

                    // pre-filter overflow_table on what differs from rib_out
                    let overflow_table: InMemoryTableState<Action<C>> = {
                        let overflow_table_flattened = overflow_table
                            .table
                            .iter()
                            .flat_map(|(net, v)| v.iter().map(move |(num, action)| (net, *num, action)));
                        let rib_out_flattened = rib_out
                            .table
                            .iter() // FUTUREWORK: since we are not interested in items only in
                                    // rib_out, could we optimize the walk further?
                            .flat_map(|(net, v)| v.iter().map(move |(num, action)| (net, *num, action)));
                        Diff::new(
                            overflow_table_flattened,
                            rib_out_flattened,
                            |(net, num, _): &(IpNet, PathId, _)| (*net, *num),
                            |(net, num, _): &(IpNet, PathId, _)| (*net, *num)
                        )
                        .filter_map(|diff_event| {
                            match diff_event {
                                DiffEvent::OnlyLeft((net, num, Action::Update(u))) => Some((net, num, Action::Update(u.clone()))),
                                // DiffEvent::OnlyLeft((net, num, Action::Withdraw)) => already withdrawn
                                DiffEvent::Both((net, num, action), (_, _, existing)) if *action != Action::Update(existing.clone()) => Some((net, num, action.clone())),
                                _ => None,

                            }
                        })
                        .collect()
                    };

                    for (net, entry) in overflow_table.table.iter() {
                        for (num, action) in entry.iter() {
                            *overflow_processed += 1;
                            match action {
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
                }
                entry = rx.recv() => {
                    if let Err(passthrough_processed) = &num_processed {
                        // We are entering an overflow phase
                        log::debug!("caught up, processed in overflow phase: {}", passthrough_processed);
                        num_processed = Ok(0);
                    }
                    let passthrough_processed = num_processed.as_mut().unwrap();
                    let Some((net, num, action)) = entry else { break; };
                    let rib_out_entry = rib_out.table.exact(&net).and_then(|entry| {
                        match entry.binary_search_by_key(&num, |(k, _)| *k) {
                            Ok(index) => Some(entry[index].1.clone()),
                            Err(_) => None,
                        }
                    });
                    let actual_action = match (action, rib_out_entry) {
                        (Action::Update(attrs), None) => Some(Action::Update(attrs)),
                        (Action::Update(attrs), Some(existing_route)) if attrs != existing_route => Some(Action::Update(attrs)),
                        (Action::Withdraw, Some(_)) => Some(Action::Withdraw),
                        _ => None,
                    };
                    let Some(actual_action) = actual_action else { continue };
                    *passthrough_processed += 1;
                    match &actual_action {
                        Action::Update(attrs) => rib_out.update_route(num, net, attrs.clone()),
                        Action::Withdraw => rib_out.withdraw_route(num, net),
                    }
                    yield (net, num, actual_action);
                }
            }
        }
        // Keeps the subscription of table events alive as long as the Stream exists
        drop(subscriber);
    }
}
