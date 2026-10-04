//! Configure and run the local Sand Post application.

mod bootstrap;
mod configuration;
mod ingestion;
mod lifecycle;

use configuration::Configuration;
use std::error::Error;
use tracing_subscriber::EnvFilter;

/// Configure and run the local web API and SMTP capture services.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let application_configuration = Configuration::from_environment()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&application_configuration.log_level)?)
        .try_init()?;
    let (storage, matcher) = bootstrap::initialize(&application_configuration)?;
    lifecycle::serve(&application_configuration, storage, matcher).await
}
