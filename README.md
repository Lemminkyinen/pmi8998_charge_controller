# pmi8998_charge_controller

A small utility for voltage-based charging control on always-on phones using a downstream `pmi8998` driver with a writable charging `status` attribute. Battery percentage is logged only; it is not trusted for control.

Charging starts below 3.75 V for 50 seconds, or immediately below 3.5 V, and recovery continues until 4.2 V. Sustained aggregate CPU usage (>= 80% for 60 seconds) also requests charging. That workload request clears after CPU usage stays <= 60% for 120 seconds. Without recovery or a workload request, charging is disabled to allow discharge.

The upper voltage limit always overrides CPU demand and rearms below or at 4.1 V. Ordinary changes have a five-minute minimum dwell; emergency starts and upper-limit stops bypass it. Monitoring continues every five seconds even during dwell. The existing voltage thresholds have **not** been retuned or validated for battery longevity; voltage varies with load and temperature, especially on an old battery.

Enabling charging does not guarantee USB bypass operation or adequate power under load. Verify battery current/voltage behavior on the actual phone. The upstream Linux 6.16 driver does not support writing `status`; this tool requires downstream support. Do not run another charging controller alongside it.

## Measurement and failure handling

Build **as your normal user**, not with `sudo`:

```sh
cargo build --release
```

First run on the phone without changing charging:

```sh
./target/release/pmi8998_charge_controller --dry-run
```

After reviewing measurements and the limitations below, real control uses the already-built binary with elevated privileges:

```sh
sudo ./target/release/pmi8998_charge_controller
```

Do not use `sudo cargo run -r`: `sudo` normally selects root's Rust environment, which may have no default toolchain even though your own user can build successfully. An error saying `rustup could not choose a version of cargo` in this situation is a build-environment error, not a controller failure. You do not need to install or configure Rust for root. Only the controller binary needs elevated privileges to write sysfs and its log file. Stopping the controller does not restore a previous charging setting; verify the desired setting afterward.

Dry-run prints readings and simulated decisions every five seconds, never writes sysfs, and does not require root if the readings are accessible. It cannot simulate how the real battery would respond to a charging change. Stop it with Ctrl+C. Real control (without `--dry-run`) requires root and logs to `/var/log/charge-control.log`, with samples every minute and every transition logged. Configure external log rotation (for example, logrotate with `copytruncate`) and your service supervisor's automatic restart policy before unattended use.

Voltage is required. Temporary failures are retried every five seconds; after three consecutive critical sample failures, the controller requests charging OFF until fresh readings return. **This deliberately favors avoiding uncontrolled charging over uptime and may eventually discharge the phone.** Failed writes are retried, not assumed successful. USB `online` is optional telemetry, not a charging permission: some drivers describe the active power path rather than physical cable presence. A false or unavailable value never blocks voltage recovery or workload requests, and changes do not reset voltage latches or cooldowns. Optional percentage, battery current, USB input current, status, and temperature readings do not stop voltage control. Missing CPU readings remove the workload request without interrupting low-voltage recovery.

Versions with the old `online=false` gate could remain disabled indefinitely, even below the emergency voltage. Do not continue a discharge test to diagnose this. Stop heavy workloads, allow a hot phone to cool, and use a known-good charger. After updating and restarting the controller, verify an actual ON request and subsequent recovery at idle before testing load. A successful sysfs write is only a request: if current/status/voltage show no recovery, investigate the cable, charger, kernel, and battery instead of trusting the requested state. If power cannot be restored, shut the phone down rather than intentionally deep-discharging it. Requesting ON cannot supply power when no charger is connected.

Temperature is currently **telemetry only**, not a software charging interlock. Hardware thermal protections must remain enabled. Battery-current direction must be verified on the phone before interpreting its sign. Gather idle/load measurements with charging both enabled and disabled before tuning voltages or adding battery-specific thermal limits. No controller can prevent swelling of a failing battery; stop using a swollen or overheating battery.

### Supervised measurement sequence

Capture a short baseline on the actual phone:

```sh
./target/release/pmi8998_charge_controller --dry-run | tee charge-measurements.log
```

Record when the real charging setting or workload changes; dry-run decisions do not make those changes. A dry-run observer may run alongside the existing controller, but changing its real setting for an experiment requires first stopping any service that would overwrite that setting. Do not run two write-enabled controllers. Keep a way to restore charging and observe the phone locally if USB also provides its network connection.

Measure these phases under supervision, allowing a few minutes for each idle baseline and only a short representative workload initially:

| Real charging setting | Workload | What to learn |
| --- | --- | --- |
| Enabled | Idle | Baseline voltage, temperature, battery and USB input current |
| Disabled | Idle | Whether the battery actually discharges while USB remains connected; voltage rebound/sag after switching |
| Disabled | Representative Docker load | Voltage sag and discharge rate under the actual host workload |
| Enabled | Same Docker load | Whether charging helps voltage/current recover without excessive heating |
| Disabled | Idle again | Whether the voltage recovers when the load ends |

Restore the original charging setting afterward. Do not deliberately discharge to the emergency threshold, start an unbounded stress test, or leave the experiment unattended. Stop on abnormal heating, rapidly collapsing voltage, or instability. Do not experiment with a swollen battery.

Interpret short traces cautiously:

- A proposed OFF action does not contradict a subsequent `Charging` status in dry-run: no write occurred.
- Battery and charger status are different readings, sampled sequentially; they can disagree or change between samples. Neither is an enable-bit readback.
- A zero USB input current reading does not prove USB supplies no power. For example, the upstream 6.16 charger driver reports zero through its measurement helper when charger status is not `Charging`. Verify downstream behavior before interpreting it.
- Percentage is informational only. A battery-current sign or a single loaded voltage does not establish remaining capacity.

Use these traces to choose operating voltages and battery-specific thermal limits; the defaults are not a calibration. Remove device identifiers and other private data before sharing logs.

Developed and tested on a OnePlus 6 `Linux enchilada 6.16.7-sdm845 #4-postmarketos-qcom-sdm845 SMP aarch64`. My phone's battery is degraded, so what works for my enchilada, might not work for yours.

### Disclaimer

This is an experimental personal project provided **as is**. Use it at your own risk. I am not responsible for any damage, data loss, battery wear, or hardware problems caused by using this tool.
