use axum::body::StreamBody;
use axum::extract::{Query as AxumQuery, State};
use axum::response::IntoResponse;
use axum::Router;
use axum::routing::get;
use crate::table::Query;
use crate::table::Table;
use futures_util::StreamExt;
use std::convert::Infallible;

async fn query<T: Table>(State(table): State<T>, AxumQuery(query): AxumQuery<Query>) -> impl IntoResponse {
    println!("request: {}", serde_json::to_string_pretty(&query).unwrap());
    let stream = table.get_routes(query)
        .map(|route| {
            let json = serde_json::to_string(&route).unwrap();
             Ok::<_, Infallible>(format!("{}\n", json))
        });
    StreamBody::new(stream)
}

fn make_api<T: Table>(table: T) -> Router {
    Router::new()
        .route("/query", get(query::<T>))
        .with_state(table)
}

pub fn start_api_server_in_new_thread<T: Table>(table: T) {

    std::thread::spawn(move || {

        loop {
            let table = table.clone();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let server = axum::Server::bind(&"[::]:3000".parse().unwrap())
                    .serve(make_api(table).into_make_service());

                if let Err(e) = server.await {
                    eprintln!("server error: {}", e);
                }
            });

            eprintln!("Restarting server after error");
        }

    });
}
