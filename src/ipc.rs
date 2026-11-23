//! Newline-delimited JSON over 127.0.0.1, shared by the app and the MCP server.
//!
//! Request:  {"cmd": "walk", "args": {"to": "cursor"}}
//! Reply:    {"ok": true, "msg": "walking to x=812"}
//!
//! A connection whose first line isn't a valid request is closed. Web pages can reach localhost, but
//! their requests start with an HTTP line, so this stops a site from smuggling a command in the body.
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

use serde_json::{json, Value};

pub const DEFAULT_PORT: u16 = 47831;
const MAX_LINE: u64 = 256 * 1024;

pub struct Request { pub cmd: String, pub args: Value, pub reply: Sender<(bool, String)> }

pub fn default_port() -> u16 {
    std::env::var("DESKLING_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)
}

/// Binds the port (an error means another buddy owns it) and serves on background threads.
pub fn serve(port: u16, inbox: Sender<Request>) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let inbox = inbox.clone();
            std::thread::spawn(move || { let _ = handle(conn, inbox); });
        }
    });
    Ok(())
}

fn parse(line: &[u8]) -> Option<(String, Value)> {
    let req: Value = serde_json::from_slice(line).ok()?;
    let cmd = req.get("cmd")?.as_str()?.to_string();
    let args = match req.get("args") { None | Some(Value::Null) => json!({}), Some(v) => v.clone() };
    Some((cmd, args))
}

fn handle(conn: TcpStream, inbox: Sender<Request>) -> std::io::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut writer = conn.try_clone()?;
    let mut reader = BufReader::new(conn);
    loop {
        let mut line = Vec::new();
        if std::io::Read::take(&mut reader, MAX_LINE).read_until(b'\n', &mut line)? == 0 { return Ok(()); }
        let Some((cmd, args)) = parse(&line) else {
            writer.write_all(b"{\"ok\":false,\"msg\":\"bad request\"}\n")?;
            return Ok(());
        };
        let (tx, rx) = channel();
        if inbox.send(Request { cmd, args, reply: tx }).is_err() { return Ok(()); }
        let (ok, msg) = rx.recv_timeout(Duration::from_secs(3)).unwrap_or((false, "buddy is busy, try again".into()));
        writer.write_all(format!("{}\n", json!({"ok": ok, "msg": msg})).as_bytes())?;
    }
}

/// One blocking request.
pub fn send(port: u16, cmd: &str, args: &Value) -> std::io::Result<(bool, String)> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut conn = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    conn.write_all(format!("{}\n", json!({"cmd": cmd, "args": args})).as_bytes())?;
    let mut line = String::new();
    BufReader::new(conn).read_line(&mut line)?;
    let reply: Value = serde_json::from_str(&line).map_err(|_| std::io::Error::other("buddy closed the connection"))?;
    Ok((reply["ok"].as_bool().unwrap_or(false), reply["msg"].as_str().unwrap_or("").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn a_web_request_is_refused_before_its_body_runs() {
        let (tx, rx) = channel();
        serve(47877, tx).unwrap();
        let mut c = TcpStream::connect(("127.0.0.1", 47877)).unwrap();
        c.write_all(b"POST / HTTP/1.1\r\nHost: x\r\n\r\n{\"cmd\":\"quit\"}\n").unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        assert!(out.contains("bad request"));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
