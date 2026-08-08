use baukit_telemetry::{
    DeploymentEnvironment, HTTP_DURATION_BUCKETS, LogFormat, ProcessKind, ServiceIdentity,
    TelemetryBuilder, TelemetryError, metrics,
};

fn identity() -> ServiceIdentity {
    ServiceIdentity::new(
        "fitness-tracker",
        ProcessKind::Api,
        "1.4.2",
        "a1b2c3d",
        DeploymentEnvironment::Local,
    )
}

#[test]
fn process_global_init_build_info_idempotence_and_shutdown() {
    // This integration-test binary isolates the non-resettable tracing and
    // metrics globals from unit tests that install scoped test subscribers.
    let telemetry = TelemetryBuilder::new(identity())
        .log_format(LogFormat::Json)
        .init()
        .expect("first initialization should succeed");

    let rendered = telemetry.prometheus_handle().render();
    assert!(rendered.contains("# TYPE build_info gauge"), "{rendered}");
    assert!(rendered.contains("commit=\"a1b2c3d\""), "{rendered}");
    assert!(rendered.contains("rust_version=\""), "{rendered}");
    assert!(rendered.contains("version=\"1.4.2\""), "{rendered}");
    assert!(rendered.contains("} 1"), "{rendered}");

    metrics::histogram!("http_request_duration_seconds").record(0.003);
    let rendered = telemetry.prometheus_handle().render();
    for bucket in HTTP_DURATION_BUCKETS {
        assert!(
            rendered.contains(&format!("le=\"{bucket}\"")),
            "missing spec bucket le=\"{bucket}\" in:\n{rendered}"
        );
    }

    assert!(matches!(
        TelemetryBuilder::new(identity()).init(),
        Err(TelemetryError::AlreadyInitialized)
    ));

    telemetry
        .shutdown()
        .expect("no-exporter shutdown should succeed");
    telemetry
        .shutdown()
        .expect("repeated shutdown should be a no-op");
}
