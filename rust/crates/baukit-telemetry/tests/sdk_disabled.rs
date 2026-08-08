use baukit_telemetry::{
    DeploymentEnvironment, LogFormat, ProcessKind, ServiceIdentity, TelemetryBuilder,
};

const CHILD_PROCESS: &str = "BAUKIT_TEST_OTEL_DISABLED_CHILD";

fn identity() -> ServiceIdentity {
    ServiceIdentity::new(
        "disabled-telemetry-test",
        ProcessKind::Api,
        "1.4.2",
        "a1b2c3d",
        DeploymentEnvironment::Production,
    )
}

#[test]
fn disabled_sdk_skips_the_trace_pipeline_but_keeps_logs_and_metrics() {
    if std::env::var_os(CHILD_PROCESS).is_none() {
        let status = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .env(CHILD_PROCESS, "1")
        .env("OTEL_SDK_DISABLED", "TrUe")
        .args([
            "--exact",
            "disabled_sdk_skips_the_trace_pipeline_but_keeps_logs_and_metrics",
            "--nocapture",
        ])
        .status()
        .expect("disabled-SDK child test should start");
        assert!(status.success(), "disabled-SDK child test failed");
        return;
    }

    // Production normally requires an OTLP endpoint. Successful initialization
    // without one proves the environment-controlled disabled path did not try
    // to build an exporter.
    let telemetry = TelemetryBuilder::new(identity())
        .log_format(LogFormat::Json)
        .init()
        .expect("disabled SDK should not require an OTLP endpoint");

    assert!(telemetry.is_otel_sdk_disabled());
    assert!(tracing::dispatcher::has_been_set());
    tracing::info!(message = "logging remains active with the OTEL SDK disabled");

    let rendered = telemetry.prometheus_handle().render();
    assert!(rendered.contains("# TYPE build_info gauge"), "{rendered}");
    assert!(rendered.contains("version=\"1.4.2\""), "{rendered}");

    telemetry
        .shutdown()
        .expect("disabled SDK shutdown should be a no-op");
}
