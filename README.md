# XEdit

A memory editor for Linux, in the spirit of Cheat Engine: attach to a running program,
look at its memory, and change it.

Small, fast, and built in layers so the AI can be an add-on rather than part of the core.

See `plan.md` for the design.

## Status

Early. Right now it can list processes and show a process's memory map.

## Build

```sh
cargo build
cargo test
cargo run -p xedit-cli -- list
```