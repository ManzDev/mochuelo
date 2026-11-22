use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_READY_ATTEMPTS: usize = 100;

/// A tiny single-document HTTP server bound to localhost.
pub struct Server {
    url: String,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    /// Start serving `html` on 127.0.0.1. Port 0 picks a free port.
    pub fn start(html: String, port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let html = Arc::new(html);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let html = Arc::clone(&html);
                        thread::spawn(move || respond(stream, &html));
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(60));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            url: format!("http://{addr}/"),
            addr,
            stop,
            handle: Some(handle),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Block until the server answers `GET /` with 200, so callers only open a
    /// browser once the page is actually being served.
    pub fn wait_ready(&self) -> std::io::Result<()> {
        let mut last_err = None;

        for _ in 0..MAX_READY_ATTEMPTS {
            match TcpStream::connect_timeout(&self.addr, Duration::from_millis(200)) {
                Ok(mut stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let request = b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
                    if stream.write_all(request).is_ok() {
                        let mut status = [0_u8; 12];
                        if stream.read_exact(&mut status).is_ok()
                            && &status[..9] == b"HTTP/1.1 "
                            && &status[9..] == b"200"
                        {
                            return Ok(());
                        }
                    }
                }
                Err(err) => last_err = Some(err),
            }
            thread::sleep(Duration::from_millis(20));
        }

        Err(last_err.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "server did not become ready",
            )
        }))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn respond(mut stream: TcpStream, html: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let request_line = {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => line,
        }
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let path = path.split('?').next().unwrap_or("/");

    let (status, content_type, body): (&str, &str, &[u8]) = match (method, path) {
        ("GET" | "HEAD", "/" | "/index.html") => (
            "200 OK",
            "text/html; charset=utf-8",
            html.as_bytes(),
        ),
        ("GET" | "HEAD", "/favicon.ico") => ("204 No Content", "image/x-icon", &[]),
        ("GET" | "HEAD", _) => ("404 Not Found", "text/plain; charset=utf-8", b"not found"),
        _ => (
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"method not allowed",
        ),
    };

    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );

    let _ = stream.write_all(head.as_bytes());
    if method != "HEAD" {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
}
