use core::router;
use tower::ServiceBuilder;
use vercel_runtime::Error;
use vercel_runtime::axum::{VercelLayer};
#[tokio::main]
async fn main() -> Result<(), Error> {
    let app = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(router());
    vercel_runtime::run(app).await
}
