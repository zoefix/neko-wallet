//! A small HTTP/1.1 server on a loopback port, for tests that need a node to
//! misbehave on cue.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// One request as the server saw it.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub at: Instant,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub enum Answer {
    Reply {
        status: u16,
        body: String,
        headers: Vec<(&'static str, String)>,
    },
    /// Read the request, then say nothing for this long.
    Hang(Duration),
}

impl Answer {
    pub fn status(status: u16, body: &str) -> Self {
        Answer::Reply {
            status,
            body: body.to_string(),
            headers: Vec::new(),
        }
    }

    pub fn ok(body: &str) -> Self {
        Answer::status(200, body)
    }

    pub fn with_header(mut self, name: &'static str, value: &str) -> Self {
        if let Answer::Reply { headers, .. } = &mut self {
            headers.push((name, value.to_string()));
        }
        self
    }
}

type Handler = dyn Fn(&Seen, usize) -> Answer + Send + Sync;

pub struct Server {
    pub url: String,
    log: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    pub fn seen(&self) -> Vec<Seen> {
        self.log.lock().unwrap().clone()
    }

    pub fn hits(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}

/// Serve `handler`, which is given each request and how many came before it.
pub async fn serve(handler: impl Fn(&Seen, usize) -> Answer + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let handler: Arc<Handler> = Arc::new(handler);
    let l = Arc::clone(&log);
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            let (l, h) = (Arc::clone(&l), Arc::clone(&handler));
            tokio::spawn(async move {
                let _ = connection(sock, l, h).await;
            });
        }
    });
    Server { url, log }
}

/// A URL nothing listens on: bound, then let go.
pub async fn refused() -> String {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    drop(l);
    url
}

async fn connection(
    mut sock: TcpStream,
    log: Arc<Mutex<Vec<Seen>>>,
    handler: Arc<Handler>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let seen = Seen {
        path,
        headers,
        body: String::from_utf8_lossy(&buf[head_end..]).to_string(),
        at: Instant::now(),
    };
    let answer = {
        let mut log = log.lock().unwrap();
        let n = log.len();
        log.push(seen.clone());
        handler(&seen, n)
    };
    match answer {
        Answer::Hang(d) => {
            tokio::time::sleep(d).await;
            Ok(())
        }
        Answer::Reply {
            status,
            body,
            headers,
        } => {
            let mut out = format!(
                "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n",
                body.len()
            );
            for (k, v) in headers {
                out.push_str(&format!("{k}: {v}\r\n"));
            }
            out.push_str("\r\n");
            out.push_str(&body);
            sock.write_all(out.as_bytes()).await?;
            sock.shutdown().await
        }
    }
}
