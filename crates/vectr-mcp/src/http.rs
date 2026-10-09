//! Optional loopback HTTP transport for the same tools (C-005, NFR-024).
//!
//! The default transport is stdio, which a host launches as a subprocess. When
//! the operator passes `--bind <address>` the server also serves MCP over
//! Streamable HTTP at `/mcp`, intended for a loopback address. The server has no
//! authentication, so it binds only what it is told and refuses a request
//! carrying a cross-origin `Origin`, which is what stops a browser page from
//! reaching a local server through DNS rebinding (NFR-024).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use crate::protocol::Server;
use crate::scope::Scope;

/// The single MCP endpoint path.
const MCP_PATH: &str = "/mcp";

/// Serves MCP over Streamable HTTP on `address` until the process is stopped.
pub fn serve(scope: &Scope, address: &str) -> io::Result<()> {
    let listener = TcpListener::bind(address)?;
    eprintln!(
        "vectr-mcp: listening on http://{}{MCP_PATH}",
        listener.local_addr()?
    );
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // A thread per connection; the server holds only the immutable
                // scope, so concurrent calls cannot corrupt one another.
                let scope = scope.clone();
                thread::spawn(move || {
                    if let Err(error) = handle(stream, &scope) {
                        eprintln!("vectr-mcp: connection error: {error}");
                    }
                });
            }
            Err(error) => eprintln!("vectr-mcp: accept error: {error}"),
        }
    }
    Ok(())
}

/// Handles one connection: one request, one response.
fn handle(stream: TcpStream, scope: &Scope) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    let Some(request) = read_request(&mut reader)? else {
        return Ok(());
    };

    if !origin_allowed(&request.headers) {
        return respond(
            &mut writer,
            403,
            "text/plain",
            b"cross-origin request refused",
        );
    }
    if request.path != MCP_PATH {
        return respond(&mut writer, 404, "text/plain", b"not found");
    }
    if let Some(version) = header(&request.headers, "mcp-protocol-version") {
        if !crate::protocol::supports_version(version) {
            return respond(
                &mut writer,
                400,
                "text/plain",
                b"unsupported protocol version",
            );
        }
    }
    if request.method != "POST" {
        return respond(&mut writer, 405, "text/plain", b"method not allowed");
    }

    let text = String::from_utf8_lossy(&request.body);
    let server = Server::new(scope);
    match server.handle_line(&text) {
        Some(response) => {
            let body = serde_json::to_vec(&response)?;
            respond(&mut writer, 200, "application/json", &body)
        }
        None => respond(&mut writer, 202, "application/json", b""),
    }
}

/// One decoded HTTP request.
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Reads one HTTP/1.1 request, or `None` at end of stream.
fn read_request(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Request>> {
    let mut start = String::new();
    if reader.read_line(&mut start)? == 0 {
        return Ok(None);
    }
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }

    let length = header(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;

    Ok(Some(Request {
        method,
        path,
        headers,
        body,
    }))
}

/// The first value of a header, case-insensitively.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Admits a request with no `Origin`, or a loopback one.
///
/// A browser sends `Origin` on a cross-site request; refusing any non-loopback
/// origin keeps a remote page from driving the local server (NFR-024).
fn origin_allowed(headers: &[(String, String)]) -> bool {
    let Some(origin) = header(headers, "origin") else {
        return true;
    };
    let authority = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    let authority = authority.split('/').next().unwrap_or(authority);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = authority
        .strip_prefix('[')
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or(authority));
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Writes a complete HTTP response.
fn respond(writer: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(
        writer,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    writer.write_all(body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vectr-mcp-http-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creates the temp dir");
        dir
    }

    #[test]
    fn a_loopback_origin_is_admitted_and_a_remote_one_is_not() {
        assert!(origin_allowed(&[]));
        assert!(origin_allowed(&[(
            "origin".to_string(),
            "http://127.0.0.1:8000".to_string()
        )]));
        assert!(origin_allowed(&[(
            "origin".to_string(),
            "http://localhost:8000".to_string()
        )]));
        assert!(!origin_allowed(&[(
            "origin".to_string(),
            "https://evil.example".to_string()
        )]));
    }

    #[test]
    fn a_posted_request_returns_the_json_rpc_response() {
        let dir = tempdir("post");
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
        let address = listener.local_addr().expect("address");

        let scope = Scope::new(vec![dir]);
        let worker = thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle(stream, &scope);
            }
        });

        let mut client = TcpStream::connect(address).expect("connects");
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        write!(
            client,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("writes");
        let mut response = String::new();
        client.read_to_string(&mut response).expect("reads");

        worker.join().expect("worker");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let payload = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("a body");
        let value: serde_json::Value = serde_json::from_str(payload).expect("valid JSON");
        assert_eq!(value["result"]["tools"].as_array().map(Vec::len), Some(5));
    }

    #[test]
    fn an_unsupported_protocol_version_header_is_refused() {
        let dir = tempdir("version");
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
        let address = listener.local_addr().expect("address");

        let scope = Scope::new(vec![dir]);
        let worker = thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle(stream, &scope);
            }
        });

        let mut client = TcpStream::connect(address).expect("connects");
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        write!(
            client,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nMCP-Protocol-Version: 1999-01-01\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("writes");
        let mut response = String::new();
        client.read_to_string(&mut response).expect("reads");

        worker.join().expect("worker");
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    }

    #[test]
    fn a_get_is_method_not_allowed() {
        let dir = tempdir("get");
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
        let address = listener.local_addr().expect("address");

        let scope = Scope::new(vec![dir]);
        let worker = thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle(stream, &scope);
            }
        });

        let mut client = TcpStream::connect(address).expect("connects");
        write!(
            client,
            "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .expect("writes");
        let mut response = String::new();
        client.read_to_string(&mut response).expect("reads");

        worker.join().expect("worker");
        assert!(response.starts_with("HTTP/1.1 405"), "{response}");
    }
}
