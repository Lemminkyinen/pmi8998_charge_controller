# pmi8998_charge_controller

A small utility tool for controlling charging on devices that use the `pmi8998` power supply driver. The idea is to keep the battery roughly between 25% - 80% for phones that stay on the cable. 

If the CPU usage stays high (>= 80% for ~60s), charging is also enabled so the system can draw its power from the charger instead of draining the battery.

Developed and tested on a OnePlus 6 `Linux enchilada 6.16.7-sdm845 #4-postmarketos-qcom-sdm845 SMP aarch64`. My phone's battery is degraded, so what works for my enchilada, might not work for yours.

### Disclaimer

This is an experimental personal project provided **as is**. Use it at your own risk. I am not responsible for any damage, data loss, battery wear, or hardware problems caused by using this tool.