use std::sync::Arc;

use clap::Parser;
use reqwest::{Client, StatusCode};

#[derive(Parser)]
struct Options {
    endpoint: String,
}

// event_time,request_id,request_type,params,tx_id,spends,creates

// event_time,request_id,request_type,result

async fn process_batch(client: Client, url: &str, id: &str, i: usize) -> eyre::Result<bool> {
    tracing::debug!("getting event {i}");
    let resp = client
        .get(format!("{url}/api/next_batch/{id}"))
        .send()
        .await?
        .error_for_status();

    let batch = match resp {
        Ok(r) => r.text().await?,
        Err(e) if e.status() == Some(StatusCode::NOT_FOUND) => {
            tracing::info!("Next batch not found, breaking...");
            return Ok(false);
        }
        Err(e) => return Err(e.into()),
    };

    // A batch holds several newline-separated input lines.
    for line in batch.lines().filter(|l| !l.trim().is_empty()) {
        // Extract event type from the first CSV field (0 or 1).
        let is_query = line
            .split(',')
            .nth(1) // request_id field
            .and_then(|f| f.parse::<u64>().ok())
            .is_some();
        tracing::debug!("event {i}: query={is_query}");
        // tracing::debug!("{}", &line[..30.min(line.len())]);

        // Only send a result for query events.
        if is_query {
            // Build a CSV output line: request_id,event_time,request_type,result
            // Extract request_id from the input line (5th column, 0-indexed 4).
            let event_time = line.split(',').nth(0).unwrap().trim();
            let request_id = line.split(',').nth(1).unwrap().trim();
            let request_type = line.split(',').nth(2).unwrap().trim();
            let output_line =
                format!("{event_time},{request_id},{request_type},dummy:response;");

            client
                .post(format!("{url}/api/result/{id}/{request_id}"))
                .body(output_line)
                .send()
                .await?
                .error_for_status()?;
            tracing::info!("sent result for event {i}");
        } else {
            tracing::debug!("event {i} is an input only event, skipping response");
        }
    }
    Ok(true)
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    let opts = Options::parse();
    let client = Client::new();
    let url = opts.endpoint;

    tracing::info!("starting demo client");

    // Create benchmark.
    let id: String = client
        .post(format!("{url}/api/create"))
        .json(&serde_json::json!({
            "apitoken": "pepega",
            "test": true,
            "queries": [0],
            // "max_batches": 20
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    tracing::info!("created bench {id}");

    // Start benchmark.
    client
        .post(format!("{url}/api/start/{id}"))
        .send()
        .await?
        .error_for_status()?;
    tracing::info!("started bench {id}");

    let url = Arc::new(url);
    let id = Arc::new(id);

    // Process batches.
    let mut i = 0;
    let mut pool = tokio::task::JoinSet::new();
    let mut finished = false;
    let parallelism = num_cpus::get();
    loop {
        while !finished && pool.len() < parallelism {
            let client = client.clone();
            let url = url.clone();
            let id = id.clone();

            pool.spawn(async move { process_batch(client.clone(), &url, &id, i).await });
            i += 1;
        }
        if let Some(r) = pool.join_next().await {
            match r? {
                Ok(true) => continue,
                Ok(false) => finished = true,
                Err(e) => eyre::bail!(e),
            }
        } else {
            break;
        }
    }

    // End benchmark and get report.
    let result: String = client
        .post(format!("{url}/api/end/{id}"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    tracing::info!("completed bench {id}");

    println!("{result}");

    Ok(())
}
