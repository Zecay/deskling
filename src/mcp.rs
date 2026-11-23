//! Stdio MCP server: nine small tools that forward to the running buddy over localhost.
use std::io::{BufRead, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::brain::ACTIONS;
use crate::ipc;

const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const KINDS: [&str; 14] = ["line", "curve", "rect", "ellipse", "arrow", "heart", "star", "text", "fire", "cloud", "bolt", "bomb", "platform", "clear"];
const FX: [&str; 5] = ["confetti", "fireworks", "hearts", "sparkle", "snow"];
const NAMES: [&str; 9] = ["say", "walk", "act", "leap", "draw", "turtle", "fx", "crew", "status"];

fn tool(name: &str, description: &str, mut props: Value, required: &[&str], targets: bool) -> Value {
    if targets { props["id"] = json!({"type": ["integer", "string"]}); }
    json!({"name": name, "description": description, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

pub fn tools() -> Value {
    let (num, text) = (json!({"type": "number"}), json!({"type": "string"}));
    json!([
        tool("say", "Speech bubble, optionally with an act.", json!({"text": text, "seconds": num, "act": {"type": "string", "enum": ACTIONS}}), &["text"], true),
        tool("walk", "Walk to 'cursor', 'left', 'right', 'center' or an x pixel.", json!({"to": {"type": ["string", "number"]}}), &["to"], true),
        tool("act", "Animate. chase = chase the mouse; celebrate = dance + confetti.", json!({"action": {"type": "string", "enum": ACTIONS}}), &["action"], true),
        tool("leap", "Jump in an arc to screen point x,y; lands on whatever is below.", json!({"x": num, "y": num}), &["x", "y"], true),
        tool("draw", "He flies over and draws it with a pencil. pts: [x,y] screen px: corners for shapes, any number for line/curve, 1 for text/fire(base)/cloud/bolt(top)/bomb, 2 ends for platform. fire burns buddies, cloud rains and puts fires out, bolt zaps and ignites, bomb explodes, platform can be stood on. ttl s, 0 = keep.",
             json!({"kind": {"type": "string", "enum": KINDS}, "pts": {"type": "array", "items": {"type": "array", "items": num}}, "text": text, "size": num, "color": text, "width": num, "ttl": num}), &["kind"], true),
        tool("turtle", "Draw with a Logo program: fd bk rt lt seth pu pd goto x y, color c, width w, arc deg r, circle r, repeat n [..]; :i = repeat counter, e.g. repeat 36 [fd :i*5 rt 100]. Starts at x,y (default above him), heading east.",
             json!({"prog": text, "x": num, "y": num, "color": text, "width": num, "ttl": num}), &["prog"], true),
        tool("fx", "Screen effect at x,y (default: him).", json!({"kind": {"type": "string", "enum": FX}, "x": num, "y": num}), &["kind"], true),
        tool("crew", "Add buddies (max 200; color: orange blue green purple pink yellow teal red white black), remove an id or 'all' extras, or set size 1-12 (default 4) for id or all.",
             json!({"add": num, "color": text, "remove": {"type": ["integer", "string"]}, "size": num}), &[], true),
        tool("status", "Buddies (id, mode, position), cursor and screen size. id param on other tools picks a buddy or 'all'; default is the first.", json!({}), &[], false),
    ])
}

fn launch() {
    const DETACHED: u32 = 0x0000_0008;
    const NEW_GROUP: u32 = 0x0000_0200;
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("--background").creation_flags(DETACHED | NEW_GROUP)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

fn call_app(cmd: &str, args: &Value) -> (bool, String) {
    let port = ipc::default_port();
    match ipc::send(port, cmd, args) {
        Ok(r) => return r,
        Err(e) if e.kind() != std::io::ErrorKind::ConnectionRefused => return (false, format!("buddy not reachable: {e}")),
        Err(_) => {}
    }
    launch();
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
        match ipc::send(port, cmd, args) {
            Ok(r) => return r,
            Err(e) if e.kind() != std::io::ErrorKind::ConnectionRefused => return (false, format!("buddy not reachable: {e}")),
            Err(_) => {}
        }
    }
    (false, "buddy did not start; try running deskling.exe".into())
}

fn result(text: &str, error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": error})
}

fn call_tool(name: &str, args: &Value) -> Value {
    let Some(map) = args.as_object() else { return result("error: arguments must be an object", true) };
    let clean: Map<String, Value> = map.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
    let (ok, msg) = call_app(name, &Value::Object(clean));
    result(&msg, !ok)
}

pub fn handle(msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id").filter(|v| !v.is_null())?;
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let ok = |r: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": r}));
    match method {
        "initialize" => {
            let wanted = params.get("protocolVersion").and_then(Value::as_str);
            let version = wanted.filter(|v| PROTOCOLS.contains(v)).unwrap_or(PROTOCOLS[0]);
            ok(json!({"protocolVersion": version, "capabilities": {"tools": {}}, "serverInfo": {"name": "deskling", "version": env!("CARGO_PKG_VERSION")}}))
        }
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if !NAMES.contains(&name) { return ok(result(&format!("unknown tool: {name}"), true)); }
            let args = params.get("arguments").filter(|a| !a.is_null()).cloned().unwrap_or(json!({}));
            ok(call_tool(name, &args))
        }
        _ => Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})),
    }
}

pub fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() { continue; }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg),
            Err(_) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}})),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_nine_tools_compactly() {
        let reply = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, NAMES);
        assert!(reply["result"]["tools"].to_string().len() < 4000);
    }

    #[test]
    fn unknown_methods_and_notifications() {
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let r = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "no/such"})).unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn initialize_echoes_a_known_protocol() {
        let r = handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "deskling");
    }
}
