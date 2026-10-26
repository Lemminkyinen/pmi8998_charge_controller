# pmi8998_charge_controller

A small utility for voltage-based charging control on always-on phones using a downstream `pmi8998` driver with a writable charging `status` attribute. Battery percentage is logged only; it is not trusted for control.

Charging starts below 3.75 V for 50 seconds, or immediately below 3.5 V, and recovery continues until 4.2 V. Sustained aggregate CPU usage (>= 80% for 60 seconds) also requests charging. That workload request clears after CPU usage stays <= 60% for 120 seconds. Without recovery or a workload request, charging is disabled to allow discharge.

The upper voltage limit always overrides CPU demand and rearms below or at 4.1 V. Ordinary changes have a five-minute minimum dwell; emergency starts and upper-limit stops bypass it. Monitoring continues every five seconds even during dwell. The existing voltage thresholds have **not** been retuned or validated for battery longevity; voltage varies with load and temperature, especially on an old battery.

Enabling charging does not guarantee USB bypass operation or adequate power under load. Verify battery current/voltage behavior on the actual phone. The upstream Linux 6.16 driver does not support writing `status`; this tool requires downstream support. Do not run another charging controller alongside it.

## Measurement and failure handling

Build with `cargo build --release`. First run on the phone without changing charging:

```sh
./target/release/pmi8998_charge_controller --dry-run
```

Dry-run prints readings and simulated decisions every five seconds, never writes sysfs, and does not require root if the readings are accessible. It cannot simulate how the real battery would respond to a charging change. Stop it with Ctrl+C. Real control (without `--dry-run`) requires root and logs to `/var/log/charge-control.log`, with samples every minute and every transition logged. Configure external log rotation (for example, logrotate with `copytruncate`) and your service supervisor's automatic restart policy before unattended use.

Voltage and USB `online` are required. Temporary failures are retried every five seconds; after three consecutive critical sample failures, the controller requests charging OFF until fresh readings return. **This deliberately favors avoiding uncontrolled charging over uptime and may eventually discharge the phone.** Failed writes are retried, not assumed successful. USB disconnect/reconnect resets the policy and triggers a fresh decision. Optional percentage, battery current, USB input current, status, and temperature readings do not stop voltage control. Missing CPU readings remove the workload request without interrupting low-voltage recovery.

Temperature is currently **telemetry only**, not a software charging interlock. Hardware thermal protections must remain enabled. Battery-current direction must be verified on the phone before interpreting its sign. Gather idle/load measurements with charging both enabled and disabled before tuning voltages or adding battery-specific thermal limits. No controller can prevent swelling of a failing battery; stop using a swollen or overheating battery.

Developed and tested on a OnePlus 6 `Linux enchilada 6.16.7-sdm845 #4-postmarketos-qcom-sdm845 SMP aarch64`. My phone's battery is degraded, so what works for my enchilada, might not work for yours.

### Disclaimer

This is an experimental personal project provided **as is**. Use it at your own risk. I am not responsible for any damage, data loss, battery wear, or hardware problems caused by using this tool.
