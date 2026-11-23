//! Deskling: Clawd and up to 199 friends on your Windows desktop, drawn on the GPU.
//!
//!     deskling                start
//!     deskling --quit         ask a running buddy to close
//!     deskling --mcp          speak MCP over stdin and stdout, for Claude Code
//!     deskling --bench 10,50,100,200   measure, then exit
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod brain;
mod crowd;
mod gpu;
mod ipc;
mod logo;
mod mcp;
mod menu;
mod props;
mod scene;
mod settings;
mod shapes;
mod region;
mod sound;
mod voice;
mod sprites;
mod studio;
mod wear;
mod win;

use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

fn value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn main() {
    unsafe { let _ = AttachConsole(ATTACH_PARENT_PROCESS); } // so messages show when started from a terminal
    // There's no console for most people, so a crash leaves a file they can attach to a bug report.
    std::panic::set_hook(Box::new(|info| {
        let path = settings::default_path().with_file_name("crash.log");
        if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
        let _ = std::fs::write(&path, format!("Deskling {}\n{info}\n", env!("CARGO_PKG_VERSION")));
        win::alert(&format!("Deskling crashed. Details are in:\n{}", path.display()));
    }));
    settings::move_old_settings();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--mcp") {
        mcp::main();
        return;
    }
    let port = value(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or_else(ipc::default_port);
    if args.iter().any(|a| a == "--quit") {
        let (ok, msg) = ipc::send(port, "quit", &serde_json::json!({})).unwrap_or((false, "buddy is not running".into()));
        println!("{msg}");
        std::process::exit(i32::from(!ok));
    }
    let options = app::Options {
        port,
        scale: value(&args, "--scale").and_then(|v| v.parse().ok()),
        max_buddies: value(&args, "--max-buddies").and_then(|v| v.parse().ok()).unwrap_or(crowd::MAX_BUDDIES),
        headless: args.iter().any(|a| a == "--headless"),
        unleashed: args.iter().any(|a| a == "--unleashed"),
        software: args.iter().any(|a| a == "--software"),
        background: args.iter().any(|a| a == "--background"),
        bench_menu: args.iter().any(|a| a == "--bench-menu"),
        bench: value(&args, "--bench").map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect()),
    };
    std::process::exit(app::run(options));
}
