use anyhow::anyhow;
use chrono::Utc;
use log::LevelFilter;
use log4rs::Config;
use log4rs::append::console::ConsoleAppender;
use log4rs::append::file::FileAppender;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use mimalloc::MiMalloc;
use nix::unistd::Uid;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use std::{fs, thread};

mod policy;
use policy::{Decision, Policy};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const LOG_FILE: &str = "/var/log/charge-control.log";
const BATTERY_VOLTAGE: &str = "/sys/class/power_supply/bq27411-0/voltage_now";
const BATTERY_CAPACITY: &str = "/sys/class/power_supply/bq27411-0/capacity";
const BATTERY_STATUS: &str = "/sys/class/power_supply/bq27411-0/status";
const BATTERY_CURRENT: &str = "/sys/class/power_supply/bq27411-0/current_now";
const BATTERY_TEMPERATURE: &str = "/sys/class/power_supply/bq27411-0/temp";
const CHARGER_ONLINE: &str = "/sys/class/power_supply/pmi8998-charger/online";
const CHARGER_STATUS: &str = "/sys/class/power_supply/pmi8998-charger/status";
const CHARGER_CURRENT: &str = "/sys/class/power_supply/pmi8998-charger/current_now";
const CPU_STAT: &str = "/proc/stat";

fn init_logger(dry_run: bool) -> anyhow::Result<()> {
    let encoder = Box::new(PatternEncoder::new(
        "{d(%Y-%m-%d %H:%M:%S%.3f)(utc)} [{l}] {m}{n}",
    ));
    let appender: Box<dyn log4rs::append::Append> = if dry_run {
        Box::new(ConsoleAppender::builder().encoder(encoder).build())
    } else {
        Box::new(FileAppender::builder().encoder(encoder).build(LOG_FILE)?)
    };

    let config = Config::builder()
        .appender(Appender::builder().build("logfile", appender))
        .build(
            Root::builder()
                .appender("logfile")
                .build(LevelFilter::Debug),
        )?;

    log4rs::init_config(config)?;
    Ok(())
}

fn read_micro_value(path: &str) -> Result<f64, anyhow::Error> {
    let raw = fs::read_to_string(path)?;
    let value = raw.trim().parse::<f64>()? * 0.000_001;
    if !value.is_finite() {
        return Err(anyhow!("Non-finite reading from {path}"));
    }
    Ok(value)
}

fn read_percentage() -> Result<u8, anyhow::Error> {
    let percentage = fs::read_to_string(BATTERY_CAPACITY)?;
    let value = percentage.trim().parse::<u8>()?;
    if value > 100 {
        return Err(anyhow!("Invalid battery percentage: {value}"));
    }
    Ok(value)
}

fn read_status(path: &str) -> Result<ChargingStatus, anyhow::Error> {
    let status = fs::read_to_string(path)?;
    Ok(ChargingStatus::from_str(&status))
}

fn read_online() -> anyhow::Result<bool> {
    let raw = fs::read_to_string(CHARGER_ONLINE)?;
    match raw.trim() {
        "0" => Ok(false),
        "1" => Ok(true),
        value => Err(anyhow!("Invalid charger online value: {value}")),
    }
}

fn read_temperature() -> anyhow::Result<f64> {
    let raw = fs::read_to_string(BATTERY_TEMPERATURE)?;
    let temperature = raw.trim().parse::<f64>()? / 10.0;
    if !temperature.is_finite() || !(-50.0..=100.0).contains(&temperature) {
        return Err(anyhow!("Invalid battery temperature: {temperature}"));
    }
    Ok(temperature)
}

/// Reads aggregate CPU times from /proc/stat and returns (idle, total) jiffies.
fn read_cpu_times() -> Result<(u64, u64), anyhow::Error> {
    let stat = fs::read_to_string(CPU_STAT)?;
    let line = stat
        .lines()
        .next()
        .ok_or_else(|| anyhow!("{CPU_STAT} is empty"))?;

    let mut fields = line.split_whitespace();
    if fields.next() != Some("cpu") {
        return Err(anyhow!("Unexpected first line in {CPU_STAT}: {line}"));
    }

    let values: Vec<u64> = fields.map(|f| f.parse::<u64>()).collect::<Result<_, _>>()?;

    // user nice system idle iowait irq softirq steal
    let field = |i: usize| values.get(i).copied().unwrap_or(0);
    let idle = field(3) + field(4);
    let non_idle = field(0) + field(1) + field(2) + field(5) + field(6) + field(7);
    Ok((idle, idle + non_idle))
}

fn set_charging_bit(b: bool) -> Result<(), anyhow::Error> {
    let content = if b { b"1" } else { b"0" };
    Ok(fs::write(CHARGER_STATUS, content)?)
}

#[derive(PartialEq, Eq, Debug, Default)]
enum ChargingStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    #[default]
    Unknown,
}

impl ChargingStatus {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "charging" => Self::Charging,
            "discharging" => Self::Discharging,
            "full" => Self::Full,
            "not charging" => Self::NotCharging,
            status => {
                eprintln!("Unknown charging status: {status} for {s}");
                Self::Unknown
            }
        }
    }
}

const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Controller {
    battery_voltage: f64,
    // Informational only: the ageing battery's capacity estimate is unreliable.
    battery_percentage: Option<u8>,
    battery_status: ChargingStatus,
    charger_status: ChargingStatus,
    charger_current: Option<f64>,
    battery_current: Option<f64>,
    battery_temperature: Option<f64>,
    charger_online: bool,

    cpu_usage: Option<f64>,
    prev_idle: Option<u64>,
    prev_total: Option<u64>,
}

impl Controller {
    fn read_battery_status(&mut self) -> Result<(), anyhow::Error> {
        let voltage = read_micro_value(BATTERY_VOLTAGE)?;
        if voltage <= 0.0 || voltage > 5.0 {
            return Err(anyhow!("Invalid battery voltage: {voltage}V"));
        }
        self.battery_voltage = voltage;
        self.charger_online = read_online()?;
        self.battery_percentage = read_percentage().ok();
        self.battery_status = read_status(BATTERY_STATUS).unwrap_or_default();
        self.charger_status = read_status(CHARGER_STATUS).unwrap_or_default();
        self.charger_current = read_micro_value(CHARGER_CURRENT).ok();
        self.battery_current = read_micro_value(BATTERY_CURRENT).ok();
        self.battery_temperature = read_temperature().ok();
        Ok(())
    }

    /// Samples aggregate CPU usage in %, computed over the interval since the
    /// previous call. The first call only stores a reference point.
    fn read_cpu_usage(&mut self) -> Result<(), anyhow::Error> {
        let (idle, total) = read_cpu_times()?;
        self.cpu_usage = match (self.prev_idle, self.prev_total) {
            (Some(prev_idle), Some(prev_total)) => {
                let idle_delta = idle.saturating_sub(prev_idle) as f64;
                let total_delta = total.saturating_sub(prev_total) as f64;
                if total_delta > 0.0 {
                    Some(((1.0 - idle_delta / total_delta) * 100.0).clamp(0.0, 100.0))
                } else {
                    None
                }
            }
            _ => None,
        };
        self.prev_idle = Some(idle);
        self.prev_total = Some(total);
        Ok(())
    }

    fn apply(&self, decision: Decision, dry_run: bool) -> Result<(), anyhow::Error> {
        let to_state = if decision.enabled { "ON" } else { "OFF" };
        let now = Utc::now();
        if !dry_run {
            set_charging_bit(decision.enabled)
                .map_err(|e| anyhow!("Failed to set charger {to_state} {e}! {now}"))?;
        }
        log::info!(
            "dry_run={dry_run} charger {to_state}: {} at {:.3}V, usb_input={:?}A, capacity={:?}%, cpu={:.1}%, battery_status={:?}, charger_status={:?}",
            decision.reason,
            self.battery_voltage,
            self.charger_current,
            self.battery_percentage,
            self.cpu_usage.unwrap_or(-1.0),
            self.battery_status,
            self.charger_status,
        );
        Ok(())
    }

    fn log_sample(&self) {
        log::info!(
            "sample voltage={:.3}V, battery_current={:?}A, temperature={:?}C, usb_input={:?}A, capacity={:?}%, cpu={:?}%, online={}, battery_status={:?}, charger_status={:?}",
            self.battery_voltage,
            self.battery_current,
            self.battery_temperature,
            self.charger_current,
            self.battery_percentage,
            self.cpu_usage,
            self.charger_online,
            self.battery_status,
            self.charger_status,
        );
    }
}

fn main() -> ExitCode {
    let mut dry_run = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--help" | "-h" => {
                println!(
                    "Usage: pmi8998_charge_controller [--dry-run]\n\n--dry-run  Log readings and proposed actions to the terminal; never write sysfs."
                );
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("Unknown argument: {arg}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = init_logger(dry_run) {
        eprintln!("Failed to initialize logger: {e}");
        return ExitCode::FAILURE;
    }

    // Check for root access
    if !dry_run && !Uid::effective().is_root() {
        log::error!("Root access required for writing {}", CHARGER_STATUS);
        return ExitCode::FAILURE;
    }

    log::info!("Starting charge controller: dry_run={dry_run}");
    log::warn!(
        "Voltage thresholds are not calibrated for this battery. Software temperature inhibition is not configured; retain hardware thermal protections."
    );
    let mut controller = Controller::default();
    let mut policy = Policy::default();
    let started = Instant::now();
    let mut failed_samples = 0u32;
    let mut next_sample_log = Duration::ZERO;
    let mut was_online = None;

    loop {
        let now = started.elapsed();
        let decision = if let Err(e) = controller.read_battery_status() {
            failed_samples = failed_samples.saturating_add(1);
            policy.interrupt_samples();
            controller.cpu_usage = None;
            controller.prev_idle = None;
            controller.prev_total = None;
            log::error!("Critical sample failed ({failed_samples}): {e}");
            // Do not charge indefinitely using stale readings. This trades
            // availability for battery protection when critical sensors fail.
            if failed_samples >= 3 {
                policy.force_off("critical sensors unavailable; readings are stale")
            } else {
                None
            }
        } else {
            if failed_samples > 0 {
                log::info!("Critical sensors recovered");
                failed_samples = 0;
            }
            if let Err(e) = controller.read_cpu_usage() {
                log::warn!("CPU reading unavailable: {e}");
                controller.cpu_usage = None;
                controller.prev_idle = None;
                controller.prev_total = None;
            }
            if dry_run || now >= next_sample_log {
                controller.log_sample();
                next_sample_log = now + Duration::from_secs(60);
            }
            if was_online != Some(controller.charger_online) {
                log::info!("Charger online={}", controller.charger_online);
                policy = Policy::default();
                was_online = Some(controller.charger_online);
            }
            if controller.charger_online {
                policy.evaluate(controller.battery_voltage, controller.cpu_usage, now)
            } else {
                // Re-evaluate from fresh state on reconnect; no repeated writes
                // to an absent power source.
                None
            }
        };
        if let Some(decision) = decision {
            match controller.apply(decision, dry_run) {
                Ok(()) => policy.applied(decision, now),
                Err(err) => log::error!("{err}; retrying on the next sample"),
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests;
