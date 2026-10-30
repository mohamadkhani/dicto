use mdict_rs::config::static_path;
use mdict_rs::formats::detect;
use mdict_rs::registry;
use mdict_rs::settings::enabled_mdx;

mod handlers;

use std::error::Error;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::handlers::{handle_lucky, handle_query};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::registry()
        .with(EnvFilter::new("info"))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Build indexes for all configured dictionaries.
    for path in enabled_mdx() {
        match detect(&path) {
            Some(dict) => {
                if let Err(e) = dict.build_index(false) {
                    warn!("indexing failed for {path}: {e}");
                }
            }
            None => warn!("unrecognised format: {path}"),
        }
    }

    // Load the global registry so query calls are served.
    registry::reload();

    let static_dir = ServeDir::new(static_path()?);

    let app = Router::new()
        .route("/query", post(handle_query))
        .route("/lucky", get(handle_lucky))
        .fallback_service(static_dir)
        .layer(TraceLayer::new_for_http());

    let port = 8181;
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8181").await.unwrap();
    info!("app serve on http://localhost:{}", port);
    axum::serve(listener, app).await?;

    Ok(())
}
