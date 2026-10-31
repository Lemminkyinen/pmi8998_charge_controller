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
fn upper_rearm_resumes_load_without_off_cooldown() {
    // Latched high voltage remains OFF; once <=4.1V with established high load,
    // re-enable immediately (no 5-min dwell). Online changes must not affect it.
    let mut policy = Policy::default();
    poll(&mut policy, 3.9, Some(99.0), Some(true), 0);
    poll(&mut policy, 3.9, Some(99.0), Some(true), 60);
    assert!(
        !poll(&mut policy, 4.2, Some(99.0), Some(false), 70)
            .unwrap()
            .enabled
    );
    // Still within latch at 4.19V -> no decision
    assert_eq!(poll(&mut policy, 4.19, Some(99.0), Some(true), 74), None);
    // Rearm at 4.1V with established heavy load -> immediate ON (no dwell)
    let on = poll(&mut policy, 4.1, Some(99.0), Some(true), 75).unwrap();
    assert!(on.enabled);
    assert_eq!(on.reason, "sustained CPU load");
    // Online changes don't affect the decision (stays ON, no repeated writes)
    assert_eq!(poll(&mut policy, 4.1, Some(99.0), None, 365), None);
    assert_eq!(poll(&mut policy, 4.1, Some(99.0), Some(false), 370), None);
}

#[test]
fn repeated_workload_intervals_enable_without_off_cooldown() {
    // After workload ON then quiet OFF, another sustained busy interval
    // enables at 60s even inside 300s since OFF.
    let mut policy = Policy::default();
    // First workload interval -> ON at 60s
    apply(&mut policy, 3.9, Some(90.0), 0);
    assert!(apply(&mut policy, 3.9, Some(90.0), 60).unwrap().enabled);
    // Quiet down: heavy_load clears after 120s of <=60% CPU
    apply(&mut policy, 3.9, Some(10.0), 70);
    apply(&mut policy, 3.9, Some(10.0), 180); // quiet_since=70, 180-70=110 < 120
    apply(&mut policy, 3.9, Some(10.0), 190); // quiet_since=70, 190-70=120 >= 120 -> heavy_load=false
    // Still ON (within MIN_ON_TIME of 300s since t=60)
    assert_eq!(apply(&mut policy, 3.9, Some(10.0), 300), None);
    assert_eq!(apply(&mut policy, 3.9, Some(10.0), 359), None);
    // OFF after MIN_ON_TIME
    assert!(!apply(&mut policy, 3.9, Some(10.0), 360).unwrap().enabled);
    // Second workload interval starts at 400s (40s after OFF)
    // Should enable at 460s (60s into new interval), not 660s (300s after OFF)
    apply(&mut policy, 3.9, Some(90.0), 400);
    assert_eq!(apply(&mut policy, 3.9, Some(90.0), 459), None);
    let on = apply(&mut policy, 3.9, Some(90.0), 460).unwrap();
    assert!(on.enabled);
    assert_eq!(on.reason, "sustained CPU load");
}

#[test]
fn low_voltage_recovery_after_upper_off_starts_at_50s_not_300s() {
    // After upper-limit OFF, sustained 3.7V starts recovery at 50s, not 300s.
    let mut policy = Policy::default();
    apply(&mut policy, 3.9, Some(90.0), 0);
    assert!(apply(&mut policy, 3.9, Some(90.0), 60).unwrap().enabled);
    assert!(!apply(&mut policy, NORMAL_STOP_V, None, 70).unwrap().enabled);
    apply(&mut policy, 4.1, None, 80);
    apply(&mut policy, 3.7, None, 80);
    assert_eq!(apply(&mut policy, 3.7, None, 129), None);
    let on = apply(&mut policy, 3.7, None, 130).unwrap();
    assert!(on.enabled);
    assert_eq!(on.reason, "low-voltage recovery");
}

#[test]
fn recovery_resumes_promptly_after_force_off_not_300s() {
    // After force_off for stale sensors during recovery, fresh 3.8V resumes
    // recovery promptly, not 300s. Fresh >3.5V prevents masking by emergency
    // bypass.
    let mut policy = Policy::default();
    apply(&mut policy, 3.7, None, 0);
    assert!(apply(&mut policy, 3.7, None, 50).unwrap().enabled);
    policy.interrupt_samples();
    let stop = policy.force_off("sensors failed").unwrap();
    assert!(!stop.enabled);
    policy.applied(stop, at(100));
    // Fresh 3.8V reading (above emergency, above normal start) resumes recovery
    let on = apply(&mut policy, 3.8, None, 110).unwrap();
    assert!(on.enabled);
    assert_eq!(on.reason, "low-voltage recovery");
}

#[test]
fn light_load_off_respects_min_on_time_but_upper_bypasses() {
    // Ordinary light-load OFF still respects minimum ON time, but
    // upper-voltage stop can bypass it.
    let mut policy = Policy::default();
    // Establish heavy load -> ON at 60s
    apply(&mut policy, 3.9, Some(90.0), 0);
    assert!(apply(&mut policy, 3.9, Some(90.0), 60).unwrap().enabled);
    // Quiet down immediately -> still ON (within MIN_ON_TIME)
    assert_eq!(apply(&mut policy, 3.9, Some(10.0), 100), None);
    assert_eq!(apply(&mut policy, 3.9, Some(10.0), 359), None);
    // After 300s ON time -> OFF for light load
    assert!(!apply(&mut policy, 3.9, Some(10.0), 360).unwrap().enabled);
    // Reset: new ON via heavy load
    apply(&mut policy, 3.9, Some(90.0), 400);
    assert!(apply(&mut policy, 3.9, Some(90.0), 460).unwrap().enabled);
    // Upper voltage -> OFF immediately (bypasses MIN_ON_TIME)
    assert!(
        !apply(&mut policy, NORMAL_STOP_V, Some(90.0), 470)
            .unwrap()
            .enabled
    );
}

#[test]
fn failed_on_write_retry_produced_next_poll_if_not_applied() {
    // Failed ON write retry inside recent OFF interval still produces next
    // poll if not applied.
    let mut policy = Policy::default();
    // Start recovery ON at emergency voltage
    let on = policy.evaluate(3.4, None, at(0)).unwrap();
    assert!(on.enabled);
    policy.applied(on, at(0));
    // Upper-voltage OFF
    let off = policy.evaluate(4.2, None, at(5)).unwrap();
    assert!(!off.enabled);
    policy.applied(off, at(5));
    // Low voltage starts at t10, qualifies at t60 (50s delay)
    policy.evaluate(3.7, None, at(10));
    let on2 = policy.evaluate(3.7, None, at(60)).unwrap();
    assert!(on2.enabled);
    assert_eq!(on2.reason, "low-voltage recovery");
    // Don't acknowledge -> retry on next poll
    assert_eq!(policy.evaluate(3.7, None, at(65)), Some(on2));
    // Acknowledge
    policy.applied(on2, at(65));
    // Now no more retries
    assert_eq!(policy.evaluate(3.7, None, at(70)), None);
}
