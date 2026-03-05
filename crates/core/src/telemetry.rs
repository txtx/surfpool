//! Prometheus metrics for Surfpool
//!
//! Feature `prometheus` enables a `/metrics` HTTP endpoint

use std::time::SystemTime;

#[cfg(feature = "prometheus")]
mod instrumented {
    use std::sync::{Once, OnceLock};

    use opentelemetry::{
        KeyValue,
        metrics::{Counter, Gauge, Meter, MeterProvider},
    };
    use opentelemetry_sdk::{Resource, metrics::SdkMeterProvider};
    use prometheus::Encoder;

    pub use super::*;

    static INIT: Once = Once::new();
    static METRICS: OnceLock<SurfpoolMetrics> = OnceLock::new();
    static METER_PROVIDER: OnceLock<SdkMeterProvider> = OnceLock::new();

    pub struct SurfpoolMetrics {
        slot: Gauge<u64>,
        epoch: Gauge<u64>,
        slot_index: Gauge<u64>,
        transactions_count: Gauge<u64>,
        transactions_processed_total: Gauge<u64>,
        uptime_seconds: Gauge<u64>,
        ws_subscriptions_total: Gauge<u64>,
        ws_signature_subscriptions: Gauge<u64>,
        ws_account_subscriptions: Gauge<u64>,
        ws_slot_subscriptions: Gauge<u64>,
        ws_logs_subscriptions: Gauge<u64>,
        transactions_processed: Counter<u64>,
    }

    impl SurfpoolMetrics {
        fn new(meter: Meter) -> Self {
            Self {
                slot: meter
                    .u64_gauge("surfpool_slot")
                    .with_description("Current slot height")
                    .build(),
                epoch: meter
                    .u64_gauge("surfpool_epoch")
                    .with_description("Current epoch")
                    .build(),
                slot_index: meter
                    .u64_gauge("surfpool_slot_index")
                    .with_description("Slot index within epoch")
                    .build(),
                transactions_count: meter
                    .u64_gauge("surfpool_transactions_count")
                    .with_description("Number of transactions in storage")
                    .build(),
                transactions_processed_total: meter
                    .u64_gauge("surfpool_transactions_processed_total")
                    .with_description("Total processed transactions")
                    .build(),
                uptime_seconds: meter
                    .u64_gauge("surfpool_uptime_seconds")
                    .with_description("Time since start in seconds")
                    .build(),
                ws_subscriptions_total: meter
                    .u64_gauge("surfpool_ws_subscriptions_total")
                    .with_description("Total WebSocket subscriptions")
                    .build(),
                ws_signature_subscriptions: meter
                    .u64_gauge("surfpool_ws_signature_subscriptions")
                    .with_description("Signature subscriptions count")
                    .build(),
                ws_account_subscriptions: meter
                    .u64_gauge("surfpool_ws_account_subscriptions")
                    .with_description("Account subscriptions count")
                    .build(),
                ws_slot_subscriptions: meter
                    .u64_gauge("surfpool_ws_slot_subscriptions")
                    .with_description("Slot subscriptions count")
                    .build(),
                ws_logs_subscriptions: meter
                    .u64_gauge("surfpool_ws_logs_subscriptions")
                    .with_description("Logs subscriptions count")
                    .build(),
                transactions_processed: meter
                    .u64_counter("surfpool_transactions_processed")
                    .with_description("Transactions processed counter")
                    .build(),
            }
        }

        pub fn record_svm_state(
            &self,
            slot: u64,
            epoch: u64,
            slot_index: u64,
            transactions_count: usize,
            transactions_processed: u64,
            start_time: SystemTime,
            signature_subs: usize,
            account_subs: usize,
            slot_subs: usize,
            logs_subs: usize,
        ) {
            let uptime_secs = SystemTime::now()
                .duration_since(start_time)
                .unwrap_or_default()
                .as_secs();

            self.slot.record(slot, &[]);
            self.epoch.record(epoch, &[]);
            self.slot_index.record(slot_index, &[]);
            self.transactions_count
                .record(transactions_count as u64, &[]);
            self.transactions_processed_total
                .record(transactions_processed, &[]);
            self.uptime_seconds.record(uptime_secs, &[]);

            let total_subs = (signature_subs + account_subs + slot_subs + logs_subs) as u64;
            self.ws_subscriptions_total.record(total_subs, &[]);
            self.ws_signature_subscriptions
                .record(signature_subs as u64, &[]);
            self.ws_account_subscriptions
                .record(account_subs as u64, &[]);
            self.ws_slot_subscriptions.record(slot_subs as u64, &[]);
            self.ws_logs_subscriptions.record(logs_subs as u64, &[]);
        }

        pub fn increment_transactions_processed(&self, count: u64) {
            self.transactions_processed.add(count, &[]);
        }
    }

    pub fn init_prometheus(
        service_name: &str,
        bind_addr: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let service_name_owned = service_name.to_string();
        let bind_addr_owned = bind_addr.to_string();

        let mut result = Ok(());

        INIT.call_once(|| {
            // Create prometheus registry
            let registry = prometheus::Registry::new();

            // Create prometheus exporter using the new 0.28 API
            // In 0.28, opentelemetry-prometheus uses a different approach
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

            // Build resource using 0.28 API
            let resource = Resource::builder()
                .with_attributes(vec![KeyValue::new("service.name", service_name_owned)])
                .build();

            // Build meter provider using 0.28 API
            let provider = SdkMeterProvider::builder()
                .with_resource(resource)
                .with_reader(exporter)
                .build();

            let meter = provider.meter("surfpool-core");
            let metrics = SurfpoolMetrics::new(meter);

            if METER_PROVIDER.set(provider).is_err() {
                result = Err("Meter provider already initialized".into());
                return;
            }

            if METRICS.set(metrics).is_err() {
                result = Err("Metrics already initialized".into());
                return;
            }

            // Spawn HTTP server in a blocking thread
            std::thread::spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("Failed to create tokio runtime: {}", e);
                        return;
                    }
                };

                rt.block_on(async {
                    let registry_clone = registry.clone();

                    // Build axum 0.8 router with new path syntax
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
                            eprintln!("Failed to bind: {}", e);
                            return;
                        }
                    };
                    if let Err(e) = axum::serve(listener, app).await {
                        eprintln!("Server error: {}", e);
                    }
                });
            });
        });

        result
    }

    pub fn metrics() -> &'static SurfpoolMetrics {
        METRICS
            .get()
            .expect("telemetry not initialized. Call init_prometheus() first")
    }

    pub fn shutdown() {
        if let Some(provider) = METER_PROVIDER.get() {
            let _ = provider.shutdown();
        }
    }
}

#[cfg(not(feature = "prometheus"))]
mod instrumented {
    use super::*;

    pub struct SurfpoolMetrics;

    impl SurfpoolMetrics {
        pub fn record_svm_state(
            &self,
            _slot: u64,
            _epoch: u64,
            _slot_index: u64,
            _transactions_count: usize,
            _transactions_processed: u64,
            _start_time: SystemTime,
            _signature_subs: usize,
            _account_subs: usize,
            _slot_subs: usize,
            _logs_subs: usize,
        ) {
        }
        pub fn increment_transactions_processed(&self, _count: u64) {}
    }

    pub fn init_prometheus(
        _service_name: &str,
        _bind_addr: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    pub fn metrics() -> SurfpoolMetrics {
        SurfpoolMetrics
    }

    pub fn shutdown() {}
}

pub use instrumented::*;

pub fn init_from_config(enabled: bool, bind_addr: &str) -> Result<(), String> {
    #[cfg(feature = "prometheus")]
    {
        if !enabled {
            log::info!("Prometheus metrics disabled");
            return Ok(());
        }
        log::info!("Starting Prometheus metrics on {}", bind_addr);
        init_prometheus("surfpool", bind_addr)
            .map_err(|e| format!("Prometheus init failed: {}", e))?;
    }
    #[cfg(not(feature = "prometheus"))]
    {
        if enabled {
            log::warn!(
                "Prometheus enabled but feature not compiled in. Rebuild with --features prometheus"
            );
        }
    }
    Ok(())
}
