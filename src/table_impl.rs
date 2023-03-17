use cidr::IpCidr;
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

impl PostgresTable {
    async fn table_up(&self, table_key: TableSelector) {
        let client = self.pool.get().await.unwrap();
        let statement = client.prepare_cached(r#"
          INSERT INTO route_tables
            VALUES (DEFAULT, $1, NOW())
            RETURNING id
        "#).await.unwrap();

        let id = {
            let table_key = serde_json::to_value(&table_key).unwrap();
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
        todo!()
        // select table_key ->> 'from_client' as from_client, prefix, routes.started_at, route_tables.ended_at, attrs from routes join route_tables on routes.table_id = route_tables.id where '2a0f:4ac0::/32' >>= prefix limit 100 ;
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
