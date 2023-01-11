mod table;
mod table_impl;
mod bmp_collector;

use futures_util::StreamExt;
use std::convert::Infallible;
use hyper::{Body, Response, Server};
use hyper::service::{make_service_fn, service_fn};
use table::Table;
use table_impl::InMemoryTable;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {

    let table: InMemoryTable = Default::default();

    {
        let table = table.clone();

        std::thread::spawn(move || {

            loop {
                let table = table.clone();
                let make_service = make_service_fn(move |_conn| {
                    let table = table.clone();
                    async move {
                        Ok::<_, Infallible>(service_fn(move |req| {
                            let query = serde_urlencoded::from_str(req.uri().query().unwrap()).unwrap();
                            println!("{}", serde_json::to_string_pretty(&query).unwrap());
                            let table = table.clone();
                            async move {
                                let resp = {
                                    let stream = table.get_routes(query)
                                        .map(|route| Ok::<_, Infallible>(serde_json::to_string(&route).unwrap()));
                                    Response::new(Body::wrap_stream(stream))
                                };
                                Ok::<_, Infallible>(resp)
                            }
                        }))
                    }
                });

                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async move {
                    let server = Server::bind(&"[::]:3000".parse().unwrap())
                        .serve(make_service);

                    if let Err(e) = server.await {
                        eprintln!("server error: {}", e);
                    }
                });

                eprintln!("Restarting server after error");
            }

        });
    }

    bmp_collector::run(table).await
}

