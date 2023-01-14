mod table;
mod table_impl;
mod bmp_collector;
mod api;


#[tokio::main]
async fn main() -> anyhow::Result<()> {

    let table: table_impl::InMemoryTable = Default::default();

    api::start_api_server_in_new_thread(table.clone());

    bmp_collector::run(table).await
}

