//! OpenTelemetry baseline (PRD-OTEL-01, 02, 03).
//!
//! Transport: OTLP/HTTP-protobuf (gRPC 회피 — 빌드 +30~40초, 바이너리 +4~6MB).
//! 호환: Tempo / Jaeger / Honeycomb / Grafana Cloud (4318 HTTP 표준).
//!
//! Resource: service.name=openxgram-core, service.version, deployment.environment.
//! Propagator: W3C tracecontext + baggage.
//!
//! 사용:
//!   let _guard = init_tracer("https://otlp.endpoint:4318")?;
//!   tracing::info!(target: "openxgram", "daemon started");
//!   // _guard drop 시 batch flush

use anyhow::{Context, Result};
use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

/// OTLP endpoint env — 표준 OTel 변수 + project specific.
pub const ENV_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
pub const SERVICE_NAME: &str = "openxgram-core";

pub struct OtelGuard {
    provider: SdkTracerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // batch flush 시도 — 실패해도 panic 금지
        let _ = self.provider.shutdown();
    }
}

/// init OTel tracer + propagator. endpoint 미지정 시 OTEL_EXPORTER_OTLP_ENDPOINT env 우선,
/// 그것도 없으면 None 반환 (OTel 비활성). 활성 시 tracing::subscriber 와 결합 권장.
pub fn init_tracer(endpoint: Option<&str>) -> Result<Option<OtelGuard>> {
    let endpoint_str = match endpoint {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => match std::env::var(ENV_OTLP_ENDPOINT) {
            Ok(s) if !s.is_empty() => s,
            _ => return Ok(None),
        },
    };

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint_str.clone())
        .build()
        .with_context(|| format!("OTLP exporter init 실패 ({endpoint_str})"))?;

    let resource = Resource::builder()
        .with_attribute(opentelemetry::KeyValue::new("service.name", SERVICE_NAME))
        .with_attribute(opentelemetry::KeyValue::new(
            "service.version",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_attribute(opentelemetry::KeyValue::new(
            "deployment.environment",
            std::env::var("XGRAM_ENV").unwrap_or_else(|_| "production".into()),
        ))
        .build();

    let provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());

    tracing::info!(endpoint = %endpoint_str, "OpenTelemetry tracer initialized");
    Ok(Some(OtelGuard { provider }))
}

/// instrument 가 적용되어야 할 hot path 함수 docs (PRD-OTEL-02).
/// 실제 #[instrument] 어트리뷰트는 각 crate 의 함수 정의부에 위치.
///
/// - openxgram_vault::vault_get / vault_put — 자격증명 latency, 실패율
/// - openxgram_memory::recall_top_k — 임베딩 + sqlite-vec hot path
/// - openxgram_memory::embedder.encode — CPU 병목
/// - openxgram_payment::submit::send_raw — 결제 신뢰성, tx_hash 속성
/// - openxgram_memory::reflect_all / pattern.classify — 야간 작업
/// - openxgram_transport::send_envelope — 전송 계층 비교 (IPC/HTTP/Nostr)
pub const HOT_PATHS: &[&str] = &[
    "vault.get",
    "vault.put",
    "memory.recall_top_k",
    "memory.embed",
    "payment.submit",
    "memory.reflect",
    "memory.classify",
    "transport.send",
];

/// MeterProvider 등록 — Prometheus pull (기존 /v1/metrics) 와 병행.
/// metrics_exporter_endpoint 미지정 시 None — pull 만 사용.
pub fn init_meter(
    endpoint: Option<&str>,
) -> Result<Option<opentelemetry_sdk::metrics::SdkMeterProvider>> {
    let endpoint_str = match endpoint {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => match std::env::var(ENV_OTLP_ENDPOINT) {
            Ok(s) if !s.is_empty() => s,
            _ => return Ok(None),
        },
    };
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(endpoint_str.clone())
        .build()
        .with_context(|| format!("OTLP metric exporter init 실패 ({endpoint_str})"))?;

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .build();

    opentelemetry::global::set_meter_provider(provider.clone());
    Ok(Some(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_tracer_returns_none_without_endpoint() {
        std::env::remove_var(ENV_OTLP_ENDPOINT);
        let g = init_tracer(None).unwrap();
        assert!(g.is_none());
    }

    #[test]
    fn init_tracer_uses_explicit_endpoint() {
        // 잘못된 endpoint 로도 init 자체는 성공 — actual export 시점에 실패
        let g = init_tracer(Some("http://localhost:14318")).unwrap();
        assert!(g.is_some());
    }

    #[test]
    fn hot_paths_documented() {
        assert!(HOT_PATHS.len() >= 6, "최소 6 함수 instrument 대상");
        for p in HOT_PATHS {
            assert!(!p.is_empty());
        }
    }
}
