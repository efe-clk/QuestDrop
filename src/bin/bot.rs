//! questdrop-bot: B-phase adapter. Polls the outbox table (same Postgres)
//! and delivers events through the configured sink.
//!
//! BOT_SINK=log (default) prints structured lines.
//! BOT_SINK=webhook + BOT_WEBHOOK_URL=<discord webhook> posts to Discord.
//! PUBLISHER_INTERVAL_SECS controls the poll cadence (default 5).

use std::time::Duration;

use questdrop::{
    notify::{LogSink, Sink, WebhookSink},
    publisher,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    dotenvy::dotenv().ok();

    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for the bot");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("bot database must be reachable");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations must apply");

    let sink: Box<dyn Sink> = match std::env::var("BOT_SINK").as_deref() {
        Ok("webhook") => {
            let hook = std::env::var("BOT_WEBHOOK_URL").expect("BOT_WEBHOOK_URL must be set");
            Box::new(WebhookSink::new(hook))
        }
        _ => Box::new(LogSink),
    };
    let interval = std::env::var("PUBLISHER_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);

    tracing::info!("bot polling every {interval}s");
    loop {
        tokio::select! {
            _ = shutdown() => {
                tracing::info!("bot shutting down");
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
        }
        match publisher::run_once(&pool, &*sink).await {
            Ok(0) => {}
            Ok(n) => tracing::info!("delivered {n} events"),
            Err(e) => tracing::error!(error = %e, "publisher batch failed"),
        }
    }
}

async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = term.recv() => {},
                _ = tokio::signal::ctrl_c() => {},
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
