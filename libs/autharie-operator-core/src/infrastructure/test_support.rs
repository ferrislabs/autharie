use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

/// A one-shot HTTP server that answers the scripted statuses and bodies in
/// order and hands back what it was asked.
pub struct Recorded {
    pub head: String,
    pub body: String,
}

pub fn serve(script: Vec<(u16, &'static str)>) -> (String, JoinHandle<Vec<Recorded>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in script {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 1024];
            let split = loop {
                let read = stream.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..read]);
                if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break at + 4;
                }
                assert!(read > 0, "connection closed before the headers ended");
            };
            let head = String::from_utf8_lossy(&raw[..split]).to_string();
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while raw.len() < split + length {
                let read = stream.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..read]);
            }
            let request_body = String::from_utf8_lossy(&raw[split..split + length]).to_string();
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            seen.push(Recorded {
                head,
                body: request_body,
            });
        }
        seen
    });
    (url, handle)
}
