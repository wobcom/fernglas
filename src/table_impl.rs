use cidr::IpCidr;
use std::time::Duration;
use std::sync::Arc;
use std::time::Instant;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use std::sync::Mutex;
use std::str::FromStr;
use futures_util::Stream;
use std::task::Poll;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::{NoTls, Statement};
use tokio_postgres::types::ToSql;
use std::pin::Pin;
use std::net::SocketAddr;
use ipnet::IpNet;
use async_trait::async_trait;
use log::*;

use crate::table::*;

//table_key: TableSelector,
//prefix: Cidr,

#[derive(Clone)]
pub struct PostgresTable {
    pool: Pool,
    queue: Arc<Mutex<Vec<(serde_json::Value, IpCidr, u32, serde_json::Value)>>>,
}

impl PostgresTable {
    pub async fn new(uri: &str) -> anyhow::Result<Self> {
        let pg_config = tokio_postgres::Config::from_str(uri)?;
        let mgr_config = ManagerConfig {
            recycling_method: RecyclingMethod::Fast
        };
        let mgr = Manager::from_config(pg_config, NoTls, mgr_config);
        let pool = Pool::builder(mgr).max_size(16).build().unwrap();

        let queue: Arc<Mutex<Vec<(serde_json::Value, IpCidr, u32, serde_json::Value)>>> = Default::default();

        for _ in 0..8 {
            let queue = queue.clone();
            let mut client = pool.get().await.unwrap();
            tokio::task::spawn(async move {
                let update_statement = client.prepare_cached(r#"
                  INSERT INTO routes
                    VALUES ($1, $2, $3)
                    ON CONFLICT (table_key, prefix, path_id) DO NOTHING
                "#).await.unwrap(); 

                let mut update_1024_statement = r#"
                  INSERT INTO routes
                    VALUES
                "#.to_string();
                for i in 0..1024 {
                    update_1024_statement += &format!("(${}, ${}, ${})", i * 3 + 1, i * 3 + 2, i * 3 + 3);
                    if i != 1023 {
                        update_1024_statement += ",\n";
                    }
                }
                update_1024_statement += r#"
                    ON CONFLICT (table_key, prefix, path_id) DO NOTHING
                "#;
                println!("{}", update_1024_statement);
                let update_1024_statement = client.prepare_cached(&update_1024_statement).await.unwrap();

                loop {
                    let mut buf: Vec<(serde_json::Value, IpCidr, u32, serde_json::Value)> = {
                        let mut queue = queue.lock().unwrap();
                        std::mem::take(&mut queue)
                    };
                    if buf.is_empty() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }

                    println!("{}", buf.len());
                    let start = Instant::now();
                    let iter = buf.chunks_exact(1024);
                    let remainder = iter.remainder();
                    for chunk in iter {
                        let mut params = chunk.iter().flat_map(|values| -> [&(dyn ToSql + Sync); 3] {
                            [&values.0, &values.1, &values.2]
                        }).collect::<Vec<_>>();
                        if let Err(e) = client.execute(&update_1024_statement, &params).await {
                            eprintln!("{:?}", e);
                        }
                    }

                    let values_arrs: Vec<[&(dyn ToSql + Sync); 3]> = remainder.iter().map(|values| -> [&(dyn ToSql + Sync); 3] {
                        [&values.0, &values.1, &values.2]
                    }).collect();
                    let mut all = values_arrs.iter().map(|arr| {
                        client.execute(&update_statement, arr)
                    }).collect::<FuturesUnordered<_>>();
                    while let Some(res) = all.next().await {
                        if let Err(e) = res {
                            eprintln!("{:?}", e);
                        }
                    }
                    let duration = start.elapsed();
                    println!("{:?}", duration);
                }
            });
        }

        Ok(Self { pool, queue })
    }
}

fn ipnet_to_cidr(net: IpNet) -> IpCidr {
    IpCidr::new(net.addr(), net.prefix_len()).unwrap()
}

#[async_trait]
impl Table for PostgresTable {
    async fn update_route(&self, path_id: u32, net: IpNet, table: TableSelector, route: RouteAttrs) {
        let mut client = self.pool.get().await.unwrap();

        let cidr = ipnet_to_cidr(net);
        let table_key = serde_json::to_value(table).unwrap();
        let attrs = serde_json::to_value(route).unwrap();

        loop {
            {
                let mut queue = self.queue.lock().unwrap();
                if queue.len() < 10240 {
                    queue.push((table_key, cidr, path_id, attrs));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn withdraw_route(&self, path_id: u32, net: IpNet, table: TableSelector) {
        let mut client = self.pool.get().await.unwrap();
    }

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = QueryResult> + Send>> {
        todo!()
    }

    async fn client_up(&self, client_addr: SocketAddr, client_data: Client) {
    }
    async fn client_down(&self, client_addr: SocketAddr) {
        let mut client = self.pool.get().await.unwrap();
    }

    async fn session_up(&self, session: SessionId, new_state: Session) {
    }
    async fn session_down(&self, session: SessionId, new_state: Option<Session>) {
        let mut client = self.pool.get().await.unwrap();
    }
}
