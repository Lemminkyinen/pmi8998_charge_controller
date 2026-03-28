use anyhow::anyhow;
use chrono::Utc;
use log::LevelFilter;
use log4rs::Config;
use log4rs::append::file::FileAppender;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use mimalloc::MiMalloc;
use nix::unistd::Uid;
use std::process::ExitCode;
use std::time::Duration;
use std::{fs, thread};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const LOG_FILE: &str = "/var/log/charge-control.log";
const BATTERY_VOLTAGE: &str = "/sys/class/power_supply/bq27411-0/voltage_now";
const BATTERY_CAPACITY: &str = "/sys/class/power_supply/bq27411-0/capacity";
const BATTERY_STATUS: &str = "/sys/class/power_supply/bq27411-0/status";
const _CHARGER_VOLTAGE: &str = "/sys/class/power_supply/pmi8998-charger/voltage_now";
const CHARGER_STATUS: &str = "/sys/class/power_supply/pmi8998-charger/status";
const CHARGER_CURRENT: &str = "/sys/class/power_supply/pmi8998-charger/current_now";

fn init_logger() -> anyhow::Result<()> {
    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "{d(%Y-%m-%d %H:%M:%S%.3f)(utc)} [{l}] {m}{n}",
        )))
        .build(LOG_FILE)?;

    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(
            Root::builder()
                .appender("logfile")
                .build(LevelFilter::Debug),
        )?;

    log4rs::init_config(config)?;
    Ok(())
}

fn read_milli_value(path: &str) -> Result<f64, anyhow::Error> {
    let voltage = fs::read_to_string(path)?;
    Ok(voltage.trim().parse::<f64>()? * 0.000_001f64)
}

fn read_percentage() -> Result<u8, anyhow::Error> {
    let percentage = fs::read_to_string(BATTERY_CAPACITY)?;
    Ok(percentage.trim().parse()?)
}

fn read_status(path: &str) -> Result<ChargingStatus, anyhow::Error> {
    let status = fs::read_to_string(path)?;
    Ok(ChargingStatus::from_str(&status))
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

struct SleepTime(Duration);

const EMERGENCY_START_V: f64 = 3.5;
const EMERGENCY_START_P: u8 = 25;

const NORMAL_START_V: f64 = 3.75;
const NORMAL_STOP_V: f64 = 4.1;
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const TOGGLE_INTERVAL: Duration = Duration::from_secs(300);

const REQUIRED_LOW_SAMPLES: u8 = 10;
const REQUIRED_HIGH_SAMPLES: u8 = 5;

#[derive(Default)]
struct Controller {
    low_samples: u8,
    high_samples: u8,

    battery_voltage: f64,
    battery_percentage: u8,
    battery_status: ChargingStatus,
    charger_status: ChargingStatus,
    charger_current: f64,
}

impl Controller {
    fn reset_samples(&mut self) {
        self.low_samples = 0;
        self.high_samples = 0;
    }

    fn read_battery_status(&mut self) -> Result<(), anyhow::Error> {
        self.battery_voltage = read_milli_value(BATTERY_VOLTAGE)?;
        self.battery_percentage = read_percentage()?;
        self.battery_status = read_status(BATTERY_STATUS)?;
        self.charger_status = read_status(CHARGER_STATUS)?;
        self.charger_current = read_milli_value(CHARGER_CURRENT)?;
        Ok(())
    }

    fn is_emergency(&self) -> bool {
        (self.battery_voltage < EMERGENCY_START_V || self.battery_percentage < EMERGENCY_START_P)
            && self.charger_status != ChargingStatus::Charging
    }

    fn is_lower_limit(&self) -> bool {
        self.battery_voltage < NORMAL_START_V && self.charger_status != ChargingStatus::Charging
    }

    fn is_higher_limit(&self) -> bool {
        self.battery_voltage > NORMAL_STOP_V
            && [
                ChargingStatus::Charging,
                ChargingStatus::Full,
                ChargingStatus::NotCharging,
            ]
            .contains(&self.battery_status)
            && self.battery_percentage > EMERGENCY_START_P
    }

    fn set_charging_bit(&mut self, bit: bool) -> Result<SleepTime, anyhow::Error> {
        let to_state = if bit { "ON" } else { "OFF" };
        let now = Utc::now();
        log::info!(
            "charger {to_state} at {:.3}V, {}A, {}%, battery_status={:?}, charger_status={:?}",
            self.battery_voltage,
            self.charger_current,
            self.battery_percentage,
            self.battery_status,
            self.charger_status,
        );
        self.reset_samples();
        match set_charging_bit(bit) {
            Ok(_) => Ok(SleepTime(TOGGLE_INTERVAL)),
            Err(e) => Err(anyhow!("Failed to set charger {to_state} {e}! {now}")),
        }
    }

    fn control_step(&mut self) -> Result<SleepTime, anyhow::Error> {
        if self.is_emergency() {
            log::warn!("Emergency start!");
            return self.set_charging_bit(true);
        }

        if self.is_lower_limit() {
            self.low_samples += 1;
            self.high_samples = 0;

            log::debug!(
                "lower-limit hit: voltage={:.3}V low_samples={}/{}",
                self.battery_voltage,
                self.low_samples,
                REQUIRED_LOW_SAMPLES
            );

            if self.low_samples >= REQUIRED_LOW_SAMPLES {
                return self.set_charging_bit(true);
            }
        } else if self.is_higher_limit() {
            self.high_samples += 1;
            self.low_samples = 0;

            log::debug!(
                "higher-limit hit: voltage={:.3}V high_samples={}/{}",
                self.battery_voltage,
                self.high_samples,
                REQUIRED_HIGH_SAMPLES
            );

            if self.high_samples >= REQUIRED_HIGH_SAMPLES {
                return self.set_charging_bit(false);
            }
        } else {
            if self.low_samples != 0 || self.high_samples != 0 {
                log::debug!("neutral zone: resetting counters");
            }
            self.reset_samples();
        }

        Ok(SleepTime(POLL_INTERVAL))
    }
}

fn main() -> ExitCode {
    if let Err(e) = init_logger() {
        eprintln!("Failed to initialize logger: {e}");
        return ExitCode::FAILURE;
    }

    // Check for root access
    if !Uid::effective().is_root() {
        log::error!("Root access required for writing {}", CHARGER_STATUS);
        return ExitCode::FAILURE;
    }

    log::info!("Starting charge controller!");
    let mut controller = Controller::default();

    loop {
        if let Err(e) = controller.read_battery_status() {
            log::error!("Failed to read battery status: {e}");
            return ExitCode::FAILURE;
        };

        match controller.control_step() {
            Ok(sleep_time) => thread::sleep(sleep_time.0),
            Err(err) => {
                log::error!("{err}");
                return ExitCode::FAILURE;
            }
        }
    }
}
