use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub fn init(verbose: bool) {
    let default_directive = if verbose { "debug" } else { "info" };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_directive));

    let fmt_layer = fmt::layer().with_target(true);

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init();
}
