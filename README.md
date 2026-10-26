# pmi8998_charge_controller

A small utility for voltage-based charging control on always-on phones using a downstream `pmi8998` driver with a writable charging `status` attribute. Battery percentage is logged only; it is not trusted for control.

Charging starts below 3.75 V for 50 seconds, or immediately below 3.5 V, and recovery continues until 4.2 V. Sustained aggregate CPU usage (>= 80% for 60 seconds) also requests charging. That workload request clears after CPU usage stays <= 60% for 120 seconds. Without recovery or a workload request, charging is disabled to allow discharge.

The upper voltage limit always overrides CPU demand and rearms below or at 4.1 V. Ordinary changes have a five-minute minimum dwell; emergency starts and upper-limit stops bypass it. Monitoring continues every five seconds even during dwell. The existing voltage thresholds have **not** been retuned or validated for battery longevity; voltage varies with load and temperature, especially on an old battery.

Enabling charging does not guarantee USB bypass operation or adequate power under load. Verify battery current/voltage behavior on the actual phone. The upstream Linux 6.16 driver does not support writing `status`; this tool requires downstream support. Do not run another charging controller alongside it.

Developed and tested on a OnePlus 6 `Linux enchilada 6.16.7-sdm845 #4-postmarketos-qcom-sdm845 SMP aarch64`. My phone's battery is degraded, so what works for my enchilada, might not work for yours.

### Disclaimer

This is an experimental personal project provided **as is**. Use it at your own risk. I am not responsible for any damage, data loss, battery wear, or hardware problems caused by using this tool.
