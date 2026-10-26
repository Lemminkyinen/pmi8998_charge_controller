use std::time::Duration;

pub const EMERGENCY_START_V: f64 = 3.5;
pub const NORMAL_START_V: f64 = 3.75;
pub const NORMAL_STOP_V: f64 = 4.2;
const UPPER_REARM_V: f64 = 4.1;
const LOW_DELAY: Duration = Duration::from_secs(50);
const LOAD_ENTER_DELAY: Duration = Duration::from_secs(60);
const LOAD_EXIT_DELAY: Duration = Duration::from_secs(120);
const MIN_DWELL: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub enabled: bool,
    pub reason: &'static str,
}

#[derive(Default)]
pub struct Policy {
    low_since: Option<Duration>,
    busy_since: Option<Duration>,
    quiet_since: Option<Duration>,
    heavy_load: bool,
    recovering: bool,
    upper_latched: bool,
    commanded: Option<bool>,
    last_transition: Option<Duration>,
}

fn sustained(since: &mut Option<Duration>, active: bool, now: Duration, delay: Duration) -> bool {
    if !active {
        *since = None;
        return false;
    }
    now.saturating_sub(*since.get_or_insert(now)) >= delay
}

impl Policy {
    pub fn interrupt_samples(&mut self) {
        self.low_since = None;
        self.busy_since = None;
        self.quiet_since = None;
        self.heavy_load = false;
    }

    pub fn force_off(&self, reason: &'static str) -> Option<Decision> {
        (self.commanded != Some(false)).then_some(Decision {
            enabled: false,
            reason,
        })
    }

    /// Evaluate fresh readings. `now` is monotonic time since process startup.
    /// The state machine is independent of charger `online` telemetry.
    pub fn evaluate(&mut self, voltage: f64, cpu: Option<f64>, now: Duration) -> Option<Decision> {
        if let Some(cpu) = cpu {
            if sustained(&mut self.busy_since, cpu >= 80.0, now, LOAD_ENTER_DELAY) {
                self.heavy_load = true;
            }
            if sustained(&mut self.quiet_since, cpu <= 60.0, now, LOAD_EXIT_DELAY) {
                self.heavy_load = false;
            }
        } else {
            // Missing CPU telemetry must not latch the workload override on forever.
            self.busy_since = None;
            self.quiet_since = None;
            self.heavy_load = false;
        }

        if voltage >= NORMAL_STOP_V {
            self.upper_latched = true;
            self.recovering = false;
        } else if voltage <= UPPER_REARM_V {
            self.upper_latched = false;
        }

        let emergency = voltage < EMERGENCY_START_V;
        if sustained(
            &mut self.low_since,
            voltage < NORMAL_START_V,
            now,
            LOW_DELAY,
        ) || emergency
        {
            self.recovering = true;
        }

        let decision = if self.upper_latched {
            Decision {
                enabled: false,
                reason: "upper voltage limit",
            }
        } else if emergency {
            Decision {
                enabled: true,
                reason: "emergency low voltage",
            }
        } else if self.recovering {
            Decision {
                enabled: true,
                reason: "low-voltage recovery",
            }
        } else if self.heavy_load {
            Decision {
                enabled: true,
                reason: "sustained CPU load",
            }
        } else {
            Decision {
                enabled: false,
                reason: "light load: allow discharge",
            }
        };

        if self.commanded == Some(decision.enabled) {
            return None;
        }
        // Keep sampling during the dwell period. Voltage limits bypass it.
        if !emergency
            && !self.upper_latched
            && self
                .last_transition
                .is_some_and(|last| now.saturating_sub(last) < MIN_DWELL)
        {
            return None;
        }
        Some(decision)
    }

    /// Only acknowledge writes that actually succeeded.
    pub fn applied(&mut self, decision: Decision, now: Duration) {
        // Establishing an OFF baseline at startup must not delay the first
        // low-voltage or sustained-load request by five minutes.
        if self.commanded.is_some() || decision.enabled {
            self.last_transition = Some(now);
        }
        self.commanded = Some(decision.enabled);
    }
}
