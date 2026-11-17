use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    pub content_type: Option<String>,
    pub body: String,
}

/// Minimal stand-in for xmrig's HTTP API. Accepts only "Bearer secret-token".
pub struct MockXmrig {
    pub port: u16,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl MockXmrig {
    pub fn start(summary_json: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(mut s) = stream {
                    handle(&mut s, summary_json, &log2);
                }
            }
        });
        MockXmrig { port, log }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

fn handle(s: &mut TcpStream, summary: &str, log: &Arc<Mutex<Vec<Recorded>>>) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let (mut auth, mut ctype, mut len) = (None, None, 0usize);
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':').unwrap_or((h, ""));
        match k.to_ascii_lowercase().as_str() {
            "authorization" => auth = Some(v.trim().to_string()),
            "content-type" => ctype = Some(v.trim().to_string()),
            "content-length" => len = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).unwrap();
    log.lock().unwrap().push(Recorded {
        method: method.clone(),
        path,
        auth: auth.clone(),
        content_type: ctype,
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    let (status, payload) = if auth.as_deref() != Some("Bearer secret-token") {
        ("401 Unauthorized", "")
    } else if method == "GET" {
        ("200 OK", summary)
    } else {
        ("204 No Content", "")
    };
    let _ = write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
}
