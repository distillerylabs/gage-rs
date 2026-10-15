//! A loopback HTTP server that records what libgit2's own HTTP transport
//! sends (Q-REMOTE-AUTH). libgit2 hands credentials only to its built-in
//! transport, never to a registered custom one, so Go's `gagetest://`
//! recording transport has no equivalent; this observes the token on the
//! wire instead, which is the stronger check. `std::net` only, no new
//! dependency, and nothing listens beyond 127.0.0.1.
//!
//! It never speaks the git protocol: every exchange ends in a failure the
//! test picks, which is the point. What is under test is what arrived.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// How the recorder answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// `401` with `WWW-Authenticate: Basic` to every request, the retry
    /// that carries credentials included: a token the server doesn't
    /// accept, so the exchange ends in an auth failure after one retry.
    Challenge,
    /// This status, with no body, to every request.
    Status(u16),
    /// `302` to `location` for a request with no `Authorization`, `401`
    /// otherwise.
    RedirectFirst(String),
    /// `401` until a request carries credentials, then `302` to
    /// `location`: a host that accepted the token and moved.
    RedirectAfterAuth(String),
}

/// One request as the recorder saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The request target: path and query.
    pub target: String,
    /// The `Host` header.
    pub host: String,
    /// The `Authorization` header, if any.
    pub authorization: Option<String>,
}

/// A running recorder. Stops when dropped.
pub struct HttpRecorder {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HttpRecorder {
    /// Starts a recorder on a free loopback port.
    pub fn start(answer: Answer) -> HttpRecorder {
        let listener = TcpListener::bind("127.0.0.1:0").expect("gittest: binding the recorder");
        let addr = listener.local_addr().expect("gittest: recorder address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (requests, stop) = (Arc::clone(&requests), Arc::clone(&stop));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        serve(stream, &answer, &requests);
                    }
                }
            })
        };
        HttpRecorder {
            addr,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// `http://127.0.0.1:<port>/<path>`.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{path}", self.addr)
    }

    /// The same server under the name `localhost`: a different host to
    /// gage, the same listener to the test.
    pub fn localhost_url(&self, path: &str) -> String {
        format!("http://localhost:{}/{path}", self.addr.port())
    }

    /// Every request so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    /// Every `Authorization` header that arrived.
    pub fn authorizations(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter_map(|r| r.authorization)
            .collect()
    }
}

impl Drop for HttpRecorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// `Basic base64(user:password)`, as a recorded header should read.
pub fn basic(user: &str, password: &str) -> String {
    format!("Basic {}", base64(format!("{user}:{password}").as_bytes()))
}

fn serve(stream: TcpStream, answer: &Answer, requests: &Mutex<Vec<Request>>) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let (mut host, mut authorization) = (String::new(), None);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => host = value.trim().to_string(),
                "authorization" => authorization = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }
    let authed = authorization.is_some();
    requests.lock().unwrap().push(Request {
        method,
        target,
        host,
        authorization,
    });

    let challenge = "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"gittest\"\r\n";
    let response = match answer {
        Answer::Challenge => challenge.to_string(),
        Answer::Status(code) => format!("HTTP/1.1 {code} {}\r\n", reason(*code)),
        Answer::RedirectFirst(location) if !authed => {
            format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\n")
        }
        Answer::RedirectAfterAuth(location) if authed => {
            format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\n")
        }
        Answer::RedirectFirst(_) | Answer::RedirectAfterAuth(_) => challenge.to_string(),
    };
    let mut stream = stream;
    let _ = stream
        .write_all(format!("{response}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
    let _ = stream.flush();
}

fn reason(code: u16) -> &'static str {
    match code {
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded, "{plain:?}");
        }
        assert_eq!(basic("gage", "tok"), "Basic Z2FnZTp0b2s=");
    }

    #[test]
    fn the_recorder_answers_and_records() {
        let recorder = HttpRecorder::start(Answer::Status(403));
        let mut stream = TcpStream::connect(recorder.addr).unwrap();
        stream
            .write_all(b"GET /x HTTP/1.1\r\nHost: h\r\nAuthorization: Basic abc\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert_eq!(response, "HTTP/1.1 403 Forbidden\r\n");
        assert_eq!(
            recorder.requests(),
            [Request {
                method: "GET".into(),
                target: "/x".into(),
                host: "h".into(),
                authorization: Some("Basic abc".into()),
            }]
        );
    }
}
