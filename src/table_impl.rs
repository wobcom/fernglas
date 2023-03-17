use cidr::{IpCidr, IpInet};
use futures_util::FutureExt;
use std::time::Instant;
use std::net::IpAddr;
use std::time::SystemTime;
use std::time::Duration;
use std::sync::Arc;
use std::collections::HashMap;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use std::sync::Mutex;
use std::str::FromStr;
use futures_util::Stream;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;
use std::pin::Pin;
use std::net::SocketAddr;
use ipnet::IpNet;
use async_trait::async_trait;
use log::*;

use crate::table::*;
use crate::compressed_attrs::*;

//table_key: TableSelector,
//prefix: Cidr,

#[derive(Default)]
pub struct PostgresTableState {
    table_ids: HashMap<TableSelector, i32>,
    queue: Vec<QueueEntry>,
}
#[derive(Clone)]
pub struct PostgresTable {
    pool: Pool,
    state: Arc<Mutex<PostgresTableState>>,
    caches: Arc<Mutex<Caches>>,
}

enum QueueEntry {
    Update {
        time: SystemTime,
        table_id: i32,
        net: IpNet,
        path_id: u32,
        attrs: Arc<CompressedRouteAttrs>,
    },
    Withdraw {
        time: SystemTime,
        table_id: i32,
        net: IpNet,
        path_id: u32,
    },
}

#[autometrics::autometrics]
pub fn count_db_insert() {
}

impl PostgresTable {
    pub async fn new(uri: &str) -> anyhow::Result<Self> {
        let pg_config = tokio_postgres::Config::from_str(uri)?;
        let mgr_config = ManagerConfig {
            recycling_method: RecyclingMethod::Fast
        };
        let mgr = Manager::from_config(pg_config, NoTls, mgr_config);

        let this = Self {
            pool: Pool::builder(mgr).max_size(32).build().unwrap(),
            caches: Default::default(),
            state: Default::default()
        };

        {
            let client = this.pool.get().await.unwrap();
            client.execute(r#"
                update route_tables
                set ended_at = r.greatest
                from (
                    select route_tables.id, greatest(max(routes.started_at), max(routes.ended_at))
                    from route_tables
                    join routes on route_tables.id = routes.table_id
                    where route_tables.ended_at is null
                    group by route_tables.id
                ) as r
                where route_tables.id = r.id
            "#, &[]).await.unwrap();
            client.execute("TRUNCATE temp_routes", &[]).await.unwrap();
        }
        // persist threads
        //for thread_num in 0..2 {
        {
            let thread_num = 0;
            let mut client = this.pool.get().await.unwrap();
            tokio::task::spawn(async move {
                loop {
                    debug!("THREAD {} persisting data", thread_num);
                    let start = Instant::now();
                    let tx = client.build_transaction().isolation_level(tokio_postgres::IsolationLevel::RepeatableRead).start().await.unwrap();
                    for query in &[
                        "CALL add_missing_attrs()",
                        "CALL process_updates()",
                        "CALL process_withdraws()",
                        "CALL delete_processed()"
                    ] {
                        let start2 = Instant::now();
                        if let Err(e) = tx.execute(*query, &[]).await {
                            println!("{}", e);
                        }
                        let took2 = start2.elapsed();
                        debug!("THREAD {} {} took {:?}", thread_num, query, took2);
                    }
                    let start2 = Instant::now();
                    tx.commit().await.unwrap();
                    let took2 = start2.elapsed();
                    debug!("THREAD {} commit took {:?}", thread_num, took2);

                    let took = start.elapsed();
                    if let Some(rest) = Duration::from_millis(15000).checked_sub(took) {
                        tokio::time::sleep(rest).await;
                    }
                }
            });
        }
        for _ in 0..8 {
            let state = this.state.clone();
            let caches = this.caches.clone();
            let client = this.pool.get().await.unwrap();
            tokio::task::spawn(async move {
                let update_statement = client.prepare_cached(r#"
                  INSERT INTO temp_routes (table_id, prefix, path_id, started_at, med, local_pref, nexthop, as_path, communities, large_communities, ended_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                "#).await.unwrap(); 

                let mut update_2048_statement = r#"
                  INSERT INTO temp_routes (table_id, prefix, path_id, started_at, med, local_pref, nexthop, as_path, communities, large_communities, ended_at)
                    VALUES
                "#.to_string();
                for i in 0..2048 {
                    update_2048_statement += "(";
                    for j in 1..12 {
                        update_2048_statement += &format!("${}", i * 11 + j);
                        if j != 11 {
                            update_2048_statement += ", ";
                        }
                    }
                    update_2048_statement += ")";
                    if i != 2047 {
                        update_2048_statement += ",\n";
                    }
                }
                let update_2048_statement = client.prepare_cached(&update_2048_statement).await.unwrap(); 

                loop {
                    let buf: Vec<QueueEntry> = {
                        let mut state = state.lock().unwrap();
                        if state.queue.len() >= 2048 {
                            state.queue.drain(..2048).collect()
                        } else {
                            std::mem::take(&mut state.queue)
                        }
                    };

                    if buf.is_empty() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }

                    let buf = buf.into_iter().filter_map(|entry| {
                        match entry {
                            QueueEntry::Update { table_id, net, attrs, path_id, time } => {
                                let cidr = ipnet_to_cidr(net);
                                Some((
                                    table_id,
                                    cidr,
                                    path_id,
                                    Some(time),
                                    attrs.med,
                                    attrs.local_pref,
                                    attrs.nexthop,
                                    attrs.as_path.as_ref().map(|a| a.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(".")),
                                    attrs.communities.as_ref().map(|a| a.iter().map(|(part1, part2)| (*part1 as u32) << 16 | *part2 as u32).collect::<Vec<_>>()),
                                    attrs.large_communities.as_ref().map(|a| a.iter().flat_map(|a| [a.0, a.1, a.2]).collect::<Vec<_>>()),
                                    None as Option<SystemTime>,
                                ))
                            }
                            QueueEntry::Withdraw { table_id, net, path_id, time } => {
                                let cidr = ipnet_to_cidr(net);
                                Some((
                                    table_id,
                                    cidr,
                                    path_id,
                                    None as Option<SystemTime>,
                                    None as Option<u32>,
                                    None as Option<u32>,
                                    None as Option<IpAddr>,
                                    None as Option<String>,
                                    None as Option<Vec<u32>>,
                                    None as Option<Vec<u32>>,
                                    Some(time)
                                ))
                            }
                        }
                    }).collect::<Vec<_>>();

                    if buf.len() == 2048 {
                        let arr: Vec<&(dyn ToSql + Sync)> = buf.iter().flat_map(|values| -> [&(dyn ToSql + Sync); 11] {
                            [&values.0, &values.1, &values.2, &values.3, &values.4, &values.5, &values.6, &values.7, &values.8, &values.9, &values.10]
                        }).collect();
                        if let Err(e) = client.execute(&update_2048_statement, &arr).await {
                            eprintln!("{:?}", e);
                        }
                        for i in 0..2048 { count_db_insert() }
                        continue;
                    } else {
                        caches.lock().unwrap().remove_expired();
                    }

                    //println!("{}", buf.len());
                    //let start = Instant::now();

                    let values_arrs: Vec<[&(dyn ToSql + Sync); 11]> = buf.iter().map(|values| -> [&(dyn ToSql + Sync); 11] {
                        [&values.0, &values.1, &values.2, &values.3, &values.4, &values.5, &values.6, &values.7, &values.8, &values.9, &values.10]
                    }).collect();
                    let mut all = values_arrs.iter().map(|arr| {
                        client.execute(&update_statement, arr)
                    }).collect::<FuturesUnordered<_>>();
                    while let Some(res) = all.next().await {
                        if let Err(e) = res {
                            eprintln!("{:?}", e);
                        }
                        count_db_insert()
                    }
                    //let duration = start.elapsed();
                    //println!("{:?}", duration);
                }
            });
        }
        Ok(this)
    }
}

fn ipnet_to_cidr(net: IpNet) -> IpCidr {
    IpCidr::new(net.addr(), net.prefix_len()).unwrap()
}
fn cidr_to_ipnet(net: IpCidr) -> IpNet {
    IpNet::new(net.first_address(), net.network_length()).unwrap()
}
fn ipnet_to_inet(net: IpNet) -> IpInet {
    IpInet::new(net.addr(), net.prefix_len()).unwrap()
}

fn make_query_string(query: &Query) -> String {
    let mut i = 0;

    let mut q = r#"
        SELECT *
        FROM view_routes
    "#.to_string();

    let mut filters = vec![];

    if let Some(table_query) = query.table_query.as_ref() {
        filters.push(match table_query {
            TableQuery::Table(_) => format!("table_key = ${}", { i += 1; i }),
            TableQuery::Session(_) => format!("table_key ->> 'from_client' = ${} AND table_key ->> 'peer_address' = ${}", { i += 1; i }, { i += 1; i }),
            TableQuery::Router(_) => format!("table_key ->> 'from_client' = ${}", { i += 1; i }),
        });
    }
    if let Some(net_query) = query.net_query.as_ref() {
        filters.push(match net_query {
            NetQuery::Contains(_) => format!("prefix >>= ${}", { i += 1; i }),
            NetQuery::MostSpecific(_) => format!("prefix = (select prefix from view_routes where prefix >>= ${} order by masklen(prefix) desc limit 1)", { i += 1; i }),
            NetQuery::Exact(_) => format!("prefix = ${}", { i += 1; i }),
            NetQuery::OrLonger(_) => format!("prefix <<= ${}", { i += 1; i }),
        });
    }
    if query.as_path_regex.is_some() {
        filters.push(format!("as_path ~ ${}", { i += 1; i }));
    }

    filters.push("ended_at IS NULL".to_string());

    if !filters.is_empty() { q += &" WHERE "; }
    q += &filters.join(" AND ");

    let limits = query.limits.clone().unwrap_or_default();
    if limits.max_results != 0 {
        q += &format!(" LIMIT ${}", { i += 1; i });
    }
    //let have_max_results_per_table = limits.max_results_per_table != 0;

    q
}
fn make_query_values(query: &Query) -> Vec<Box<dyn ToSql + Send + Sync>> {
    let mut v: Vec<Box<dyn ToSql + Send + Sync>> = vec![];

    if let Some(table_query) = query.table_query.as_ref() {
        match table_query {
            TableQuery::Table(ts) => v.push(Box::new(serde_json::to_value(&ts).unwrap())),
            TableQuery::Session(SessionId { from_client, peer_address }) => {
                v.push(Box::new(from_client.to_string()));
                v.push(Box::new(peer_address.to_string()));
            },
            TableQuery::Router(from_client) => v.push(Box::new(from_client.to_string())),
        }
    }
    if let Some(net_query) = query.net_query.as_ref() {
        let x = match net_query {
            NetQuery::Contains(x) => x,
            NetQuery::MostSpecific(x) => x,
            NetQuery::Exact(x) => x,
            NetQuery::OrLonger(x) => x,
        };
        v.push(Box::new(ipnet_to_inet(*x)));
    }
    if let Some(as_path_regex) = &query.as_path_regex {
        v.push(Box::new(as_path_regex.to_string()));
    }

    let limits = query.limits.clone().unwrap_or_default();
    if limits.max_results != 0 {
        v.push(Box::new(limits.max_results as i64));
    }

    v
}

impl PostgresTable {
    async fn table_up(&self, table_key: TableSelector) {
        let client = self.pool.get().await.unwrap();
        let statement = client.prepare_cached(r#"
          INSERT INTO route_tables
            VALUES (DEFAULT, $1, NOW())
            RETURNING id
        "#).await.unwrap();

        let id = {
            let route_state = serde_json::to_value(&table_key.route_state()).unwrap();
            let mut table_key = serde_json::to_value(&table_key).unwrap();
            table_key.as_object_mut().unwrap().insert("route_state".to_string(), route_state);
            let res = client.query(&statement, &[&table_key]).await.unwrap();
            res.into_iter().next().unwrap().get(0)
        };
        let mut state = self.state.lock().unwrap();
        state.table_ids.insert(table_key, id);
    }
    async fn table_down(&self, filter_fn: impl Fn(&TableSelector) -> bool) {
        let client = self.pool.get().await.unwrap();
        let statement = client.prepare_cached(r#"
          UPDATE route_tables
            SET ended_at = NOW()
            WHERE id = $1
        "#).await.unwrap();

        let throw = {
            let mut state = self.state.lock().unwrap();
            let table_ids = std::mem::take(&mut state.table_ids);
            let (throw, keep) = table_ids.into_iter().partition(|(table_key, _)| {
                filter_fn(&table_key)
            });
            state.table_ids = keep;
            throw
        };

        for (_table_key, id) in throw {
            client.execute(&statement, &[&id]).await.unwrap();
        }
    }
}

#[async_trait]
impl Table for PostgresTable {
    #[autometrics::autometrics]
    async fn update_route(&self, path_id: u32, net: IpNet, table: TableSelector, route: RouteAttrs) {
        let compressed = self.caches.lock().unwrap().compress_route_attrs(route);

        let mut state = self.state.lock().unwrap();
        let table_id = *state.table_ids.get(&table).unwrap();
        state.queue.push(QueueEntry::Update {
            time: SystemTime::now(),
            path_id, net, table_id, attrs: compressed
        });
    }

    #[autometrics::autometrics]
    async fn withdraw_route(&self, path_id: u32, net: IpNet, table: TableSelector) {
        let mut state = self.state.lock().unwrap();
        let table_id = *state.table_ids.get(&table).unwrap();
        state.queue.push(QueueEntry::Withdraw {
            time: SystemTime::now(),
            path_id, net, table_id
        });
    }

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = QueryResult> + Send>> {
        let query_string = make_query_string(&query);
        let query_values = make_query_values(&query);
        let pool = self.pool.clone();
        Box::pin((async move {
            let client = pool.get().await.unwrap();
            let statement = client.prepare_cached(&query_string).await.unwrap();
            client.query_raw(&statement, query_values).await.unwrap()
        })
            .flatten_stream()
            .filter_map(|row| async {
                let row = match row {
                    Err(e) => {
                        println!("{}", e);
                        return None;
                    },
                    Ok(v) => v,
                };
                let table_key: serde_json::Value = row.get(0);
                let prefix: IpCidr = row.get(1);
                let started_at: SystemTime = row.get(2);
                let ended_at: Option<SystemTime> = row.get(3);
                let med: Option<u32> = row.get(4);
                let local_pref: Option<u32> = row.get(5);
                let nexthop: Option<IpInet> = row.get(6);
                let as_path: Option<&str> = row.get(7);
                let communities: Option<Vec<u32>> = row.get(8);
                let large_communities: Option<Vec<u32>> = row.get(9);

                let table: TableSelector = serde_json::from_value(table_key).unwrap();
                Some(QueryResult {
                    state: RouteState::Selected,
                    net: cidr_to_ipnet(prefix),
                    client: Client { client_name: table.client_addr().to_string() },
                    table: table,
                    session: None,
                    attrs: RouteAttrs {
                        //origin: Option<RouteOrigin>,
                        as_path: as_path.map(|x| x.split(".").filter_map(|i| (i != "").then(|| u32::from_str(i).unwrap())).collect()),
                        communities: communities.map(|x| x.into_iter().map(|i| ((i >> 16) as u16, (i & 0xff) as u16)).collect()),
                        large_communities: large_communities.map(|x| x.chunks_exact(3).map(|i| (i[0], i[1], i[2])).collect()),
                        med,
                        local_pref,
                        nexthop: nexthop.map(|x| x.address()),
                        ..Default::default()
                    },
                })
            }))
    }

    async fn client_up(&self, client_addr: SocketAddr, route_state: RouteState, client_data: Client) {
        self.table_up(TableSelector::LocRib { from_client: client_addr, route_state }).await;
    }
    async fn client_down(&self, client_addr: SocketAddr) {
        self.table_down(|table_key| {
            table_key.client_addr() == &client_addr
        }).await;
    }

    async fn session_up(&self, session: SessionId, new_state: Session) {
        for table_key in [TableSelector::PrePolicyAdjIn(session.clone()), TableSelector::PostPolicyAdjIn(session)] {
            self.table_up(table_key).await;
        }
    }
    async fn session_down(&self, session: SessionId, new_state: Option<Session>) {
        self.table_down(|table_key| {
            table_key.session_id() == Some(&session)
        }).await;
    }
}
