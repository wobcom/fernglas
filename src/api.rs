use axum::body::StreamBody;
use axum::extract::{Query as AxumQuery, State};
use axum::handler::Handler;
use axum::response::IntoResponse;
use crate::table::Query;
use crate::table::Table;
use futures_util::StreamExt;
use std::convert::Infallible;

async fn handle_request<T: Table>(State(table): State<T>, AxumQuery(query): AxumQuery<Query>) -> impl IntoResponse {
    println!("request: {}", serde_json::to_string_pretty(&query).unwrap());
    let stream = table.get_routes(query)
        .map(|route| Ok::<_, Infallible>(serde_json::to_string(&route).unwrap()));
    StreamBody::new(stream)
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
                    .serve(handle_request::<T>.with_state(table).into_make_service());

                if let Err(e) = server.await {
                    eprintln!("server error: {}", e);
                }
            });

            eprintln!("Restarting server after error");
        }

    });
}
