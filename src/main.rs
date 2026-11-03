use axum::Router;
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use micro_challenger::{cli, html, json, Context};

#[derive(clap::Parser)]
struct Options {
    #[clap(short('t'), long)]
    load_tar: Option<PathBuf>,

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
        println!(
            "{bench_id}|{}|{}",
            report.bench_name,
            report
                .queries
                .get(&0)
                .map(|l| l.timings.iter().flatten().count())
                .unwrap_or_default()
        );
    }
}

fn csv_results(ctx: &Context) {
    println!("bench_id,bench_name,count,missing,throughput,latency_mean,latency_min,latency_p99");
    for (bench_id, report) in ctx.history().expect("failed to fetch history").into_iter() {
        let bench_name = report.bench_name;
        let Some(q) = report.queries.get(&0) else {
            continue;
        };

        let micro_challenger::QueryReport {
            count,
            missing,
            throughput,
            latency_mean,
            latency_min,
            latency_p99,
            ..
        } = q;

        println!(
            "{bench_id},{bench_name},{count},{missing},{throughput},{latency_mean},{latency_min},{latency_p99}",
        );
    }
}
