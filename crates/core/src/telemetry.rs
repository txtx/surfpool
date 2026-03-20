//! Prometheus metrics for Surfpool
//!
//! Feature `prometheus` enables a `/metrics` HTTP endpoint

#[cfg(feature = "prometheus")]
mod instrumented {
    use std::sync::{Once, OnceLock};

    use opentelemetry::{
        KeyValue,
        metrics::{Gauge, Meter, MeterProvider},
    };
    use opentelemetry_sdk::{Resource, metrics::SdkMeterProvider};
    use prometheus::Encoder;
    use surfpool_types::SurfpoolStatus;

    static INIT: Once = Once::new();
    static METRICS: OnceLock<SurfpoolMetrics> = OnceLock::new();
    static METER_PROVIDER: OnceLock<SdkMeterProvider> = OnceLock::new();
    static SHUTDOWN_TX: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();

    pub struct SurfpoolMetrics {
        uptime_seconds: Gauge<u64>,
        transactions_processed_count: Gauge<u64>,
    }

    impl SurfpoolMetrics {
        fn new(meter: Meter) -> Self {
            Self {
                uptime_seconds: meter
                    .u64_gauge("surfpool_uptime_seconds")
                    .with_description("Time since start in seconds")
                    .build(),
                transactions_processed_count: meter
                    .u64_gauge("surfpool_transactions_processed_count")
                    .with_description("Total processed transactions")
                    .build(),
            }
        }

        fn record_snapshot(&self, status: &SurfpoolStatus) {
            self.uptime_seconds.record(status.uptime_ms / 1000, &[]);
            self.transactions_processed_count
                .record(status.transactions_processed, &[]);
        }
    }

    /// Record a snapshot if metrics are initialized. Safe to call unconditionally.
    pub fn try_record_snapshot(status: &SurfpoolStatus) {
        if let Some(m) = METRICS.get() {
            m.record_snapshot(status);
        }
    }

    pub fn init_prometheus(
        service_name: &str,
        bind_addr: &str,
        handle: &tokio::runtime::Handle,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let service_name_owned = service_name.to_string();
        let bind_addr_owned = bind_addr.to_string();
        let mut result = Ok(());

        INIT.call_once(|| {
            let registry = prometheus::Registry::new();
            let exporter = match opentelemetry_prometheus::exporter()
                .with_registry(registry.clone())
                .build()
            {
                Ok(exp) => exp,
                Err(e) => {
                    result = Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
                    return;
                }
            };

            let resource = Resource::builder()
                .with_attributes(vec![KeyValue::new("service.name", service_name_owned)])
                .build();

            let provider = SdkMeterProvider::builder()
                .with_resource(resource)
                .with_reader(exporter)
                .build();

            let meter = provider.meter("surfpool-core");
            let metrics = SurfpoolMetrics::new(meter);

            if let Err(e) = METER_PROVIDER.set(provider) {
                result = Err(format!("Meter provider already initialized: {:?}", e).into());
                return;
            }
            if METRICS.set(metrics).is_err() {
                result = Err("Metrics already initialized".into());
                return;
            }

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
            let _ = SHUTDOWN_TX.set(shutdown_tx);

            handle.spawn(async move {
                let registry_clone = registry.clone();
                let app = axum::Router::new().route(
                    "/metrics",
                    axum::routing::get(move || {
                        let reg = registry_clone.clone();
                        async move {
                            let encoder = prometheus::TextEncoder::new();
                            let metric_families = reg.gather();
                            let mut buffer = vec![];
                            if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
                                return (
                                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                    format!("Failed to encode: {}", e),
                                );
                            }
                            let body = String::from_utf8(buffer)
                                .unwrap_or_else(|_| "Invalid UTF8".to_string());
                            (axum::http::StatusCode::OK, body)
                        }
                    }),
                );
                let listener = match tokio::net::TcpListener::bind(&bind_addr_owned).await {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("Failed to bind metrics endpoint: {}", e);
                        return;
                    }
                };
                if let Err(e) = axum::serve(listener, app)
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.changed().await;
                    })
                    .await
                {
                    eprintln!("Metrics server error: {}", e);
                }
            });
        });

        result
    }

    pub fn shutdown() {
        if let Some(tx) = SHUTDOWN_TX.get() {
            let _ = tx.send(true);
        }
        if let Some(provider) = METER_PROVIDER.get() {
            let _ = provider.shutdown();
        }
    }
}

#[cfg(feature = "prometheus")]
pub use instrumented::*;

#[cfg(feature = "prometheus")]
pub fn init_from_config(bind_addr: &str, handle: &tokio::runtime::Handle) -> Result<(), String> {
    log::info!("Starting Prometheus metrics on {}", bind_addr);
    init_prometheus("surfpool", bind_addr, handle)
        .map_err(|e| format!("Prometheus init failed: {}", e))
}
