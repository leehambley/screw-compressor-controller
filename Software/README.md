# compressor-controller

ESP32 Embassy/Rust replacement controller for a Ceccato ES3000-based screw
compressor. Full spec and current open items are in `CLAUDE.md` — start
there.

## Resuming in Claude Code

1. Move this folder to wherever you want the real project to live, and
   `git init` it if you haven't already.
2. Open Claude Code in this directory (`claude` from the terminal, or the
   desktop app). It reads `CLAUDE.md` automatically for context — you
   shouldn't need to re-explain the spec.
3. Toolchain, if not already set up:
   ```
   cargo install espup
   espup install
   source ~/export-esp.sh   # or the equivalent for your shell
   cargo install espflash
   ```
4. `cargo build` to check the skeleton compiles for your `esp-hal`/
   `embassy` versions — pin versions in `Cargo.toml` are current as of
   this scaffold but worth checking against latest before building out
   further.

## Status

Skeleton only: module structure, config constants, and sensor scaling
functions are filled in from the agreed spec. `state_machine::Controller`
transition logic and `main()` task wiring are stubs — that's the next
work. See "Not yet done / open items" in `CLAUDE.md`.
