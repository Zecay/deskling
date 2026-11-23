# Contributing

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Build

You need Windows 10 or 11 and [Rust](https://rustup.rs) with the MSVC toolchain.

```powershell
cargo test
cargo run --release
```

To build an `.exe` to share, run `./scripts/build-release.ps1`. It keeps your PC's folder names out of the file.

## Useful flags

| Flag | What it does |
|---|---|
| `--bench 10,50,100,200` | Measures performance, then exits |
| `--bench-menu` | Keeps the menu open during a bench |
| `--software` | Uses the software renderer, like a PC with no GPU |
| `--unleashed` | Removes limits for this run, for stress tests |
| `--port 47999` | Runs a second copy beside your normal one |
| `--quit` | Closes a running copy |

## Where things live

| File in `src` | Purpose |
|---|---|
| `brain.rs` | One buddy's behavior, with no Windows calls |
| `crowd.rs` | Buddies together: piles, crashes, chats |
| `menu.rs` | Radial and dropdown menus |
| `studio.rs` | The Deskling Studio settings window |
| `settings.rs` | Saved choices, validated on load |
| `scene.rs`, `gpu.rs`, `shaders.hlsl` | Drawing |
| `app.rs` | Main loop, mouse, tray and hotkeys |
| `mcp.rs`, `ipc.rs` | Claude Code connection |

## Pull requests

- Keep each pull request to one change.
- Run `cargo test`, and add a test when you change behavior.
- With 200 buddies, the bench should stay at 60 fps.
- Don't add telemetry or network calls.
