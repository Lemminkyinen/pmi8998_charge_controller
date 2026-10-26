use super::*;

#[test]
fn percentage_does_not_control_emergency_charging() {
    for percentage in [None, Some(0), Some(25), Some(100)] {
        let controller = Controller {
            battery_voltage: 3.9,
            battery_percentage: percentage,
            ..Controller::default()
        };
        assert!(!controller.is_emergency());
    }
}

#[test]
fn low_voltage_requests_charging_even_with_full_capacity_estimate() {
    let controller = Controller {
        battery_voltage: 3.4,
        battery_percentage: Some(100),
        ..Controller::default()
    };
    assert!(controller.is_emergency());
}

#[test]
fn upper_limit_is_independent_of_capacity_status_and_usb_current() {
    for percentage in [None, Some(0), Some(100)] {
        let controller = Controller {
            battery_voltage: NORMAL_STOP_V,
            battery_percentage: percentage,
            battery_status: ChargingStatus::Discharging,
            charger_current: 2.0,
            ..Controller::default()
        };
        assert!(controller.is_higher_limit());
    }
}
