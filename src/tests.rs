use super::policy::*;
use std::time::Duration;

fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

fn apply(policy: &mut Policy, voltage: f64, cpu: Option<f64>, seconds: u64) -> Option<Decision> {
    let result = policy.evaluate(voltage, cpu, at(seconds));
    if let Some(decision) = result {
        policy.applied(decision, at(seconds));
    }
    result
}

#[test]
fn idle_starts_disabled_and_does_not_repeat_writes() {
    let mut policy = Policy::default();
    assert!(!apply(&mut policy, 3.9, Some(0.0), 0).unwrap().enabled);
    assert_eq!(apply(&mut policy, 3.9, Some(0.0), 5), None);
}

#[test]
fn low_voltage_requires_elapsed_time_then_recovers_until_upper_limit() {
    let mut policy = Policy::default();
    assert!(!apply(&mut policy, 3.7, None, 0).unwrap().enabled);
    assert_eq!(apply(&mut policy, 3.7, None, 49), None);
    assert!(apply(&mut policy, 3.7, None, 50).unwrap().enabled);
    assert_eq!(apply(&mut policy, 3.9, None, 55), None);
    assert!(!apply(&mut policy, NORMAL_STOP_V, None, 60).unwrap().enabled);
}

#[test]
fn emergency_bypasses_cooldown_and_does_not_depend_on_status_or_percentage() {
    let mut policy = Policy::default();
    apply(&mut policy, 3.9, None, 0);
    assert!(apply(&mut policy, 3.4, None, 5).unwrap().enabled);
}

#[test]
fn load_enables_charging_and_quiet_period_releases_it() {
    let mut policy = Policy::default();
    apply(&mut policy, 3.9, Some(90.0), 0);
    assert_eq!(apply(&mut policy, 3.9, Some(90.0), 59), None);
    assert!(apply(&mut policy, 3.9, Some(90.0), 60).unwrap().enabled);
    assert_eq!(apply(&mut policy, 3.9, Some(10.0), 600), None);
    assert_eq!(apply(&mut policy, 3.9, Some(10.0), 719), None);
    assert!(!apply(&mut policy, 3.9, Some(10.0), 720).unwrap().enabled);
}

#[test]
fn upper_limit_overrides_load_and_prevents_voltage_rebound_chatter() {
    let mut policy = Policy::default();
    policy.evaluate(3.9, Some(90.0), at(0));
    assert!(apply(&mut policy, 3.9, Some(90.0), 60).unwrap().enabled);
    assert!(
        !apply(&mut policy, NORMAL_STOP_V, Some(90.0), 65)
            .unwrap()
            .enabled
    );
    assert_eq!(apply(&mut policy, 4.19, Some(90.0), 400), None);
    assert!(apply(&mut policy, 4.1, Some(90.0), 405).unwrap().enabled);
}

#[test]
fn broken_cpu_telemetry_does_not_leave_workload_charging_latched() {
    let mut policy = Policy::default();
    policy.evaluate(3.9, Some(90.0), at(0));
    apply(&mut policy, 3.9, Some(90.0), 60);
    assert!(!apply(&mut policy, 3.9, None, 360).unwrap().enabled);
}

#[test]
fn failed_write_is_not_acknowledged_and_is_retried() {
    let mut policy = Policy::default();
    let first = policy.evaluate(3.9, None, at(0));
    assert!(first.is_some());
    assert_eq!(policy.evaluate(3.9, None, at(5)), first);
}

#[test]
fn interrupted_cpu_burst_does_not_count_as_sustained_load() {
    let mut policy = Policy::default();
    apply(&mut policy, 3.9, Some(90.0), 0);
    apply(&mut policy, 3.9, Some(20.0), 30);
    assert_eq!(apply(&mut policy, 3.9, Some(90.0), 300), None);
    assert_eq!(apply(&mut policy, 3.9, Some(90.0), 359), None);
    assert!(apply(&mut policy, 3.9, Some(90.0), 360).unwrap().enabled);
}

#[test]
fn interrupted_readings_restart_debounce_windows() {
    let mut policy = Policy::default();
    apply(&mut policy, 3.7, Some(90.0), 0);
    policy.interrupt_samples();
    assert_eq!(apply(&mut policy, 3.7, Some(90.0), 300), None);
    assert_eq!(apply(&mut policy, 3.7, Some(90.0), 349), None);
    assert!(apply(&mut policy, 3.7, Some(90.0), 350).unwrap().enabled);
}

#[test]
fn sensor_failure_stop_bypasses_dwell_and_only_acknowledges_success() {
    let mut policy = Policy::default();
    apply(&mut policy, 3.4, None, 0);
    let stop = policy.force_off("sensors failed").unwrap();
    assert!(!stop.enabled);
    assert_eq!(policy.force_off("sensors failed"), Some(stop));
    policy.applied(stop, at(5));
    assert_eq!(policy.force_off("sensors failed"), None);
    // Fresh critically low readings can still recover immediately.
    assert!(apply(&mut policy, 3.4, None, 10).unwrap().enabled);
}

#[test]
fn quiet_load_does_not_interrupt_low_voltage_recovery() {
    let mut policy = Policy::default();
    apply(&mut policy, 3.4, Some(90.0), 0);
    apply(&mut policy, 3.9, Some(10.0), 300);
    assert_eq!(apply(&mut policy, 3.9, Some(10.0), 420), None);
    assert!(
        !apply(&mut policy, NORMAL_STOP_V, Some(10.0), 425)
            .unwrap()
            .enabled
    );
}

#[test]
fn middle_cpu_band_holds_heavy_mode_without_chatter() {
    let mut policy = Policy::default();
    policy.evaluate(3.9, Some(90.0), at(0));
    apply(&mut policy, 3.9, Some(90.0), 60);
    assert_eq!(apply(&mut policy, 3.9, Some(70.0), 360), None);
    assert_eq!(apply(&mut policy, 3.9, Some(70.0), 600), None);
}
