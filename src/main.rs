use axum::Router;
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use micro_challenger::{Context, cli, html, json};

#[derive(clap::Parser)]
struct Options {
    #[clap(short('t'), long)]
    load_tar: Option<PathBuf>,

    #[clap(long)]
    load_csv_input: Option<PathBuf>,

    /// Output CSV file.
    #[clap(long)]
    load_csv_output: Option<PathBuf>,

    /// Maximum batch size for input stream (type-1/query events always split).
    #[clap(long, default_value = "1024")]
    max_batch_size: u32,

    /// JSON file (request type -> track name) to store as request mapping.
    #[clap(long)]
    load_request_mapping: Option<PathBuf>,

    #[clap(long)]
    list_results: bool,

    #[clap(long)]
    csv_results: bool,

    #[clap(long)]
    delete_result: Option<String>,

    /// REST soket address
    bind_address: SocketAddr,

    data: PathBuf,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "micro_challenger=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let opts = Options::parse();

    if let Some(tar) = opts.load_tar {
        cli::load_tar(&opts.data, &tar).unwrap();
        return;
    }

    if let Some(json) = &opts.load_request_mapping {
        cli::load_request_mapping(&opts.data, json).unwrap();
    }

    if let Some(input_csv) = &opts.load_csv_input
        && let Some(output_csv) = &opts.load_csv_output
    {
        cli::load_csv(&opts.data, input_csv, output_csv, opts.max_batch_size).unwrap();
        return;
    } else if opts.load_csv_input.xor(opts.load_csv_output).is_some() {
        panic!("Either specify both load_csv_input and load_csv_output or none.")
    }

    let ctx = Context::new(&opts.data).unwrap();

    if opts.list_results {
        list_results(&ctx);
    }

    if opts.csv_results {
        csv_results(&ctx);
    }

    if let Some(id) = opts.delete_result {
        if ctx
            .delete_result(&id)
            .expect("failed to delete bench result")
        {
            tracing::info!("removed bench with id {id}.");
        } else {
            tracing::warn!("bench with id {id} not found.");
        }
        return;
    }

    // ROUTES
    let app = Router::new()
        .nest("/api", json::router(ctx.clone()))
        .merge(html::router())
        .fallback((
            axum::http::StatusCode::NOT_FOUND,
            "Not found or wrong method/payload",
        ));

    tracing::info!("starting micro-challenger at {}", opts.bind_address);
    axum_server::bind(opts.bind_address)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

fn list_results(ctx: &Context) {
    for (bench_id, report) in ctx.history().expect("failed to fetch history").into_iter() {
        let bench_name = report.bench_name;
        for (query_id, qr) in &report.tracks {
            let count = qr.timings.iter().flatten().count();
            println!("{bench_id}|{bench_name}|q{query_id}|{count}");
        }
    }
}

fn csv_results(ctx: &Context) {
    println!(
        "bench_id,bench_name,query_id,count,missing,correct,wrong,throughput,latency_mean,latency_min,latency_max,latency_p99"
    );
    for (bench_id, report) in ctx.history().expect("failed to fetch history").into_iter() {
        let bench_name = report.bench_name;
        for (query_id, qr) in &report.tracks {
            let micro_challenger::TimingReport {
                count,
                missing,
                correct,
                wrong,
                throughput,
                latency_mean,
                latency_min,
                latency_max,
                latency_p99,
                ..
            } = qr;

            println!(
                "{bench_id},{bench_name},{query_id},{count},{missing},{correct},{wrong},{throughput},{latency_mean},{latency_min},{latency_max},{latency_p99}",
            );
        }
    }
}
