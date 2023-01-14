mod table;
mod table_impl;
mod bmp_collector;
mod bgpdumper;
mod bgp_collector;
mod api;


#[tokio::main]
async fn main() -> anyhow::Result<()> {

    let table: table_impl::InMemoryTable = Default::default();

    api::start_api_server_in_new_thread(table.clone());

    tokio::select! {
        val = bmp_collector::run(table.clone()) => val,
        val = bgp_collector::run(table) => val,
    }
}

