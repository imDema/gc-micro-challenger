use clap::Parser;
use reqwest::Client;

#[derive(Parser)]
struct Options {
    endpoint: String,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    let opts = Options::parse();
    let client = Client::new();
    let url = opts.endpoint;

    tracing::info!("starting demo client");
    let id = client
        .post(format!("{url}/api/create"))
        .json(&serde_json::json!({"apitoken": "pepega","test": true,"queries": [0],"max_batches": 20}))
        .send()
        .await?
        .error_for_status()?
        .json::<String>()
        .await?;
    tracing::info!("created bench {id}");

    assert!(dbg!{client
        .post(format!("{url}/api/start/{id}"))
        .send()
        .await?}
        .status()
        .is_success());
    tracing::info!("started bench {id}");

    for i in 0..20 {
        tracing::info!("getting batch {i}");
        let input = client
            .post(format!("{url}/api/next_batch/{id}"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        tracing::debug!("{input}");
        tracing::info!("sending batch {i}");
        assert!(client
            .post(format!("{url}/api/result/0/{id}/{i}"))
            .body(input)
            .send()
            .await?
            .status()
            .is_success());
    }

    let result = client
        .post(format!("{url}/api/end/{id}"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    tracing::info!("completed bench {id}");

    println!("result: {result}");

    Ok(())
}
