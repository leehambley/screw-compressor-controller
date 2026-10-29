#!/usr/bin/env python3
"""Modbus RTU test server that pretends to be the SAKO SKI-70 VFD.

Emulates the register map the controller's `src/modbus.rs` speaks, per
CLAUDE.md's "VFD Modbus" section (empirically discovered, not in the SKI-70
manual):

  - 9600 baud, 8N1, slave address 1
  - Control word:      reg 8193  (write, 0x06): 1 = run, 5 = stop
  - Freq setpoint:      reg 4097  (write, 0x06): Hz * 200
  - Monitoring:         reg 28673 (read,  0x03): running freq/100,
                         output current/100, bus voltage/10 (3 regs)
  - Freq limits:        reg 61453 (read,  0x03): min, unused, max (Hz*200)

Run against a USB-to-RS485 adapter wired to the controller's Modbus lines
in place of the real VFD:

    python3 tools/vfd_sim.py /dev/tty.usbserial-XXXX

Run with no arguments to list available serial ports. While running, type
'status', 'drop' (toggle simulated comms loss), or 'quit' at the prompt.
"""
import argparse
import struct
import sys
import threading
import time

import serial
import serial.tools.list_ports

CONTROL_WORD_REG = 8193
FREQ_SETPOINT_REG = 4097
MONITOR_BASE_REG = 28673
FREQ_MIN_REG = 61453

FUNC_READ_HOLDING = 0x03
FUNC_WRITE_SINGLE = 0x06

# VFD_ACCEL_MS / VFD_DECEL_MS in config.rs are both 5500ms, spanning the
# full 0-50Hz range.
ACCEL_HZ_PER_S = 50.0 / 5.5
DECEL_HZ_PER_S = 50.0 / 5.5


def crc16(data: bytes) -> int:
    crc = 0xFFFF
    for b in data:
        crc ^= b
        for _ in range(8):
            if crc & 1:
                crc = (crc >> 1) ^ 0xA001
            else:
                crc >>= 1
    return crc


class VfdState:
    def __init__(self, min_hz: float, max_hz: float):
        self.lock = threading.Lock()
        self.running = False
        self.setpoint_hz = 0.0
        self.freq_hz = 0.0
        self.min_hz = min_hz
        self.max_hz = max_hz
        self.drop_responses = False
        self._last_tick = time.monotonic()

    def tick(self) -> None:
        now = time.monotonic()
        with self.lock:
            dt = now - self._last_tick
            self._last_tick = now
            target = self.setpoint_hz if self.running else 0.0
            if self.freq_hz < target:
                self.freq_hz = min(target, self.freq_hz + ACCEL_HZ_PER_S * dt)
            elif self.freq_hz > target:
                self.freq_hz = max(target, self.freq_hz - DECEL_HZ_PER_S * dt)

    def set_control_word(self, value: int) -> None:
        with self.lock:
            if value == 1:
                self.running = True
            elif value == 5:
                self.running = False

    def set_setpoint_raw(self, raw: int) -> None:
        with self.lock:
            self.setpoint_hz = raw / 200.0

    def monitor_regs(self) -> list[int]:
        with self.lock:
            freq = self.freq_hz
            running = self.running
        current_a = 0.0 if freq <= 0.01 else 1.5 + freq * 0.18
        bus_v = 540.0 if (running or freq > 0.01) else 325.0
        return [
            int(round(freq * 100)) & 0xFFFF,
            int(round(current_a * 100)) & 0xFFFF,
            int(round(bus_v * 10)) & 0xFFFF,
        ]

    def limit_regs(self) -> list[int]:
        with self.lock:
            return [
                int(round(self.min_hz * 200)) & 0xFFFF,
                0,
                int(round(self.max_hz * 200)) & 0xFFFF,
            ]


def read_request(ser: serial.Serial, slave_addr: int, verbose: bool) -> bytes | None:
    addr = ser.read(1)
    if not addr or addr[0] != slave_addr:
        return None
    func = ser.read(1)
    if not func or func[0] not in (FUNC_READ_HOLDING, FUNC_WRITE_SINGLE):
        return None
    rest = ser.read(6)  # 4 param bytes + 2 CRC bytes
    if len(rest) != 6:
        return None
    frame = addr + func + rest
    if verbose:
        print(f"[rx] {frame.hex(' ')}")
    if crc16(frame[:-2]) != struct.unpack("<H", frame[-2:])[0]:
        print(f"[modbus] CRC mismatch, dropping: {frame.hex(' ')}")
        return None
    return frame


def read_registers(state: VfdState, start: int, count: int) -> list[int] | None:
    if start == MONITOR_BASE_REG and count <= 3:
        return state.monitor_regs()[:count]
    if start == FREQ_MIN_REG and count <= 3:
        return state.limit_regs()[:count]
    return None


def handle_write(state: VfdState, reg: int, value: int) -> None:
    if reg == CONTROL_WORD_REG:
        state.set_control_word(value)
        label = {1: "RUN", 5: "STOP"}.get(value, "?")
        print(f"[vfd] control word <- {value} ({label})")
    elif reg == FREQ_SETPOINT_REG:
        state.set_setpoint_raw(value)
        print(f"[vfd] setpoint <- {value / 200.0:.2f} Hz")
    else:
        print(f"[vfd] write to unmodeled register {reg} = {value}")


def serve(ser: serial.Serial, state: VfdState, slave_addr: int, verbose: bool, stop_event: threading.Event) -> None:
    while not stop_event.is_set():
        frame = read_request(ser, slave_addr, verbose)
        if frame is None:
            continue
        addr, func = frame[0], frame[1]
        if func == FUNC_READ_HOLDING:
            start, count = struct.unpack(">HH", frame[2:6])
            regs = read_registers(state, start, count)
            if regs is None:
                print(f"[modbus] unsupported read: start={start} count={count}")
                continue
            resp = bytes([addr, func, len(regs) * 2])
            resp += b"".join(struct.pack(">H", r) for r in regs)
            resp += struct.pack("<H", crc16(resp))
        else:
            reg, value = struct.unpack(">HH", frame[2:6])
            handle_write(state, reg, value)
            resp = frame  # SKI-70 echoes the write request back verbatim

        if state.drop_responses:
            print("[modbus] drop active, suppressing response (simulated comms loss)")
            continue
        ser.write(resp)
        ser.flush()
        if verbose:
            print(f"[tx] {resp.hex(' ')}")


def ticker(state: VfdState, stop_event: threading.Event) -> None:
    while not stop_event.is_set():
        state.tick()
        time.sleep(0.05)


def command_loop(state: VfdState, stop_event: threading.Event) -> None:
    print("Commands: status | drop | quit")
    while not stop_event.is_set():
        try:
            line = input().strip()
        except EOFError:
            stop_event.set()
            return
        if line == "status":
            with state.lock:
                print(
                    f"[cmd] running={state.running} "
                    f"setpoint={state.setpoint_hz:.2f}Hz freq={state.freq_hz:.2f}Hz "
                    f"drop={state.drop_responses}"
                )
        elif line == "drop":
            with state.lock:
                state.drop_responses = not state.drop_responses
            print(f"[cmd] drop_responses = {state.drop_responses}")
        elif line in ("quit", "exit"):
            stop_event.set()
        elif line:
            print(f"[cmd] unknown command: {line}")


def list_ports() -> None:
    ports = list(serial.tools.list_ports.comports())
    if not ports:
        print("No serial ports found.", file=sys.stderr)
        return
    print("Available serial ports:")
    for p in ports:
        print(f"  {p.device}  {p.description}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Modbus RTU test server emulating the SAKO SKI-70 VFD.")
    parser.add_argument("port", nargs="?", help="Serial device, e.g. /dev/tty.usbserial-XXXX")
    parser.add_argument("--baud", type=int, default=9600, help="default matches config::MODBUS_BAUD_RATE (unconfirmed placeholder)")
    parser.add_argument("--slave-addr", type=int, default=1)
    parser.add_argument("--min-hz", type=float, default=35.0)
    parser.add_argument("--max-hz", type=float, default=50.0)
    parser.add_argument("-v", "--verbose", action="store_true", help="log raw frame bytes")
    args = parser.parse_args()

    if not args.port:
        list_ports()
        print("\nRe-run with one of the above as the port argument.")
        return

    ser = serial.Serial(args.port, args.baud, bytesize=8, parity="N", stopbits=1, timeout=0.2)
    state = VfdState(args.min_hz, args.max_hz)
    stop_event = threading.Event()

    print(f"VFD sim listening on {args.port} @ {args.baud} 8N1, slave addr {args.slave_addr}")
    print(f"freq limits: {args.min_hz:.1f}-{args.max_hz:.1f} Hz")

    threads = [
        threading.Thread(target=ticker, args=(state, stop_event), daemon=True),
        threading.Thread(target=serve, args=(ser, state, args.slave_addr, args.verbose, stop_event), daemon=True),
    ]
    for t in threads:
        t.start()

    try:
        if sys.stdin.isatty():
            command_loop(state, stop_event)
        else:
            print("stdin is not a tty; skipping command prompt (Ctrl+C to stop)")
            while not stop_event.is_set():
                time.sleep(0.5)
    except KeyboardInterrupt:
        stop_event.set()
    finally:
        ser.close()


if __name__ == "__main__":
    main()
