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

/// Like `apply`, but goes through the orchestration helper used by main,
/// with an explicit charger `online` reading.
fn poll(
    policy: &mut Policy,
    voltage: f64,
    cpu: Option<f64>,
    online: Option<bool>,
    seconds: u64,
) -> Option<Decision> {
    let controller = super::Controller {
        battery_voltage: voltage,
        cpu_usage: cpu,
        charger_online: online,
        ..super::Controller::default()
    };
    let result = controller.evaluate_policy(policy, at(seconds));
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

#[test]
fn emergency_enable_at_3_4v_with_online_false() {
    // The phone trace: online=false the whole time, CPU pegged, battery
    // sagging. The emergency command must still be issued.
    let mut policy = Policy::default();
    assert!(
        !poll(&mut policy, 3.9, Some(99.0), Some(false), 0)
            .unwrap()
            .enabled
    );
    let emergency = poll(&mut policy, 3.4, Some(99.0), Some(false), 5).unwrap();
    assert!(emergency.enabled);
    assert_eq!(emergency.reason, "emergency low voltage");
}

#[test]
fn sustained_cpu_enable_is_independent_of_online_false() {
    let mut policy = Policy::default();
    // Startup establishes the OFF baseline (a real write); from then on the
    // sustained-load decision is issued exactly once.
    assert!(
        !poll(&mut policy, 3.9, Some(99.0), Some(false), 0)
            .unwrap()
            .enabled
    );
    assert!(
        poll(&mut policy, 3.9, Some(99.0), Some(false), 60)
            .unwrap()
            .enabled
    );
    // Stays enabled while the load persists, offline included; no repeated
    // write every poll.
    assert_eq!(poll(&mut policy, 3.9, Some(99.0), Some(false), 3000), None);
    assert_eq!(poll(&mut policy, 3.9, Some(99.0), Some(false), 3005), None);
}

#[test]
fn normal_low_voltage_recovery_while_online_false() {
    let mut policy = Policy::default();
    // Light load at 3.7V: the policy first establishes an OFF baseline, then
    // enables charging 50s after the low voltage is sustained.
    assert!(
        !poll(&mut policy, 3.7, Some(10.0), Some(false), 0)
            .unwrap()
            .enabled
    );
    assert!(
        poll(&mut policy, 3.7, Some(10.0), Some(false), 50)
            .unwrap()
            .enabled
    );
    // Recovery holds until the upper limit, with online still false.
    assert_eq!(poll(&mut policy, 3.9, Some(10.0), Some(false), 100), None);
    assert_eq!(poll(&mut policy, 3.9, Some(10.0), Some(false), 3000), None);
}

#[test]
fn missing_online_does_not_block_policy() {
    let mut policy = Policy::default();
    // A read failure (`None`) is not treated as offline and does not block
    // the emergency command.
    let controller = super::Controller {
        battery_voltage: 3.4,
        cpu_usage: Some(99.0),
        charger_online: None,
        ..super::Controller::default()
    };
    let d = controller.evaluate_policy(&mut policy, at(0)).unwrap();
    assert!(d.enabled);
    // It is also not treated as online; the next poll repeats until applied.
    assert_eq!(controller.evaluate_policy(&mut policy, at(5)), Some(d));
}

#[test]
fn upper_latch_survives_online_flips() {
    let mut policy = Policy::default();
    assert!(
        !poll(&mut policy, 3.9, Some(99.0), Some(true), 0)
            .unwrap()
            .enabled
    );
    assert!(
        poll(&mut policy, 3.9, Some(99.0), Some(true), 60)
            .unwrap()
            .enabled
    );
    let off = poll(&mut policy, 4.2, Some(99.0), Some(false), 70).unwrap();
    assert!(!off.enabled);
    // Upper latch keeps charging off while the voltage stays high, no matter
    // how the online bit flips.
    assert_eq!(poll(&mut policy, 4.19, Some(99.0), Some(true), 100), None);
    assert_eq!(poll(&mut policy, 4.19, Some(99.0), Some(false), 400), None);
    // The latch must survive an offline->online->offline flip too.
    assert_eq!(poll(&mut policy, 4.19, Some(99.0), Some(true), 700), None);
    assert_eq!(poll(&mut policy, 4.19, Some(99.0), Some(false), 1000), None);
    // Rearm at <= UPPER_REARM_V, online back to false.
    assert!(
        poll(&mut policy, 4.1, Some(99.0), Some(false), 1005)
            .unwrap()
            .enabled
    );
}

#[test]
fn cooldown_survives_online_changes_after_upper_rearm() {
    let mut policy = Policy::default();
    poll(&mut policy, 3.9, Some(99.0), Some(true), 0);
    poll(&mut policy, 3.9, Some(99.0), Some(true), 60);
    assert!(
        !poll(&mut policy, 4.2, Some(99.0), Some(false), 70)
            .unwrap()
            .enabled
    );
    // Rearm the voltage latch, but retain the five-minute dwell even if
    // online flips or becomes unavailable.
    assert_eq!(poll(&mut policy, 4.1, Some(99.0), Some(true), 75), None);
    assert_eq!(poll(&mut policy, 4.1, Some(99.0), None, 365), None);
    assert!(
        poll(&mut policy, 4.1, Some(99.0), Some(false), 370)
            .unwrap()
            .enabled
    );
}
