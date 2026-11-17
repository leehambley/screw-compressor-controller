# Firmware

`no_std` Rust for the ESP32-S3 on the Waveshare ESP32-S3-Touch-LCD-4:
esp-hal 1.0, Embassy on `esp-rtos`, Slint UI.

What the project is, the hardware and how the pieces fit are in the
[top-level README](../README.md). The long-form notes (measurements, decisions,
open items) are in [CLAUDE.md](CLAUDE.md).

## Build and flash

```sh
. ~/export-esp.sh        # Xtensa toolchain from `espup install --targets esp32s3`
cargo run --release      # flashes over USB-C (USB-Serial-JTAG) and shows the log
```

`cargo run --release --bin vfd-run-test` exercises the VFD on its own.

## Layout

- `src/main.rs`: start-up and the control loop
- `src/state_machine.rs`, `pid.rs`, `modbus.rs`: control, speed loop, drive comms
- `src/board/`: Waveshare board and piggyback chip drivers (see `display.rs`, `st7701/`)
- `src/ui.rs`, `ui/main.slint`: the touch UI (fonts in `ui/fonts/`)
- `src/config.rs`: all tunable constants
- `tools/`: `vfd_sim.py` (a fake SKI-70 for bench tests) and `st7701_golden.rs`
