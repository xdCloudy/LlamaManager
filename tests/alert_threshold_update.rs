use llamamanager::telemetry_alerts::{
    AlertComparator, AlertEngine, AlertEventKind, AlertPresentationState, AlertRule,
    AlertSeverity, AlertThreshold, AlertValueRange,
};
use llamamanager::telemetry_history::{
    SampleSource, SeriesIdentity, SeriesKey, TimeSeriesSample,
};

fn key() -> SeriesKey {
    SeriesKey::new(
        "gpu.temperature",
        "celsius",
        "nvidia-nvml",
        "nvmlDeviceGetTemperature",
        SeriesIdentity::new("gpu-uuid", "GPU-1").with_display_name("RTX test"),
    )
}

fn rule() -> AlertRule {
    AlertRule {
        id: "gpu-temperature-warning".to_owned(),
        metric: "gpu.temperature".to_owned(),
        source_provider: "nvidia-nvml".to_owned(),
        source_api: "nvmlDeviceGetTemperature".to_owned(),
        severity: AlertSeverity::Warning,
        comparator: AlertComparator::Above,
        threshold: AlertThreshold {
            trigger: 80.0,
            clear: 75.0,
        },
        window_ms: 2_000,
        debounce_ms: 3_000,
        min_live_samples: 3,
        valid_value_range: Some(AlertValueRange {
            min: -20.0,
            max: 150.0,
        }),
        reason: "configured sustained GPU temperature threshold".to_owned(),
    }
}

fn live(key: &SeriesKey, timestamp: u64, value: f64) -> TimeSeriesSample {
    TimeSeriesSample::live(timestamp, value, SampleSource::from_key(key)).unwrap()
}

#[test]
fn threshold_edit_preserves_active_state_until_new_clear_policy_is_satisfied() {
    let key = key();
    let mut engine = AlertEngine::new(vec![rule()]).unwrap();

    for (timestamp, value) in [(1_000, 82.0), (2_000, 83.0), (3_000, 84.0)] {
        engine.observe(&key, &live(&key, timestamp, value)).unwrap();
    }
    assert_eq!(engine.history().len(), 1);
    assert_eq!(engine.history().back().unwrap().kind, AlertEventKind::Fired);

    engine
        .update_threshold(
            "gpu-temperature-warning",
            AlertThreshold {
                trigger: 90.0,
                clear: 85.0,
            },
        )
        .unwrap();

    let still_active = engine
        .observe(&key, &live(&key, 4_000, 86.0))
        .unwrap()
        .remove(0);
    assert_eq!(still_active.state, AlertPresentationState::Active);
    assert!(still_active.transition.is_none());
    assert_eq!(engine.history().len(), 1);

    for (timestamp, value) in [(5_000, 84.0), (6_000, 83.0)] {
        let clearing = engine
            .observe(&key, &live(&key, timestamp, value))
            .unwrap()
            .remove(0);
        assert_eq!(clearing.state, AlertPresentationState::Clearing);
        assert!(clearing.transition.is_none());
    }

    let resolved = engine
        .observe(&key, &live(&key, 7_000, 82.0))
        .unwrap()
        .remove(0);
    assert_eq!(resolved.state, AlertPresentationState::Inactive);
    let event = resolved.transition.expect("active alert should resolve truthfully");
    assert_eq!(event.kind, AlertEventKind::Resolved);
    assert_eq!(event.evidence.trigger_threshold, 90.0);
    assert_eq!(event.evidence.clear_threshold, 85.0);
    assert_eq!(engine.history().len(), 2);
}
