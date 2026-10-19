//! The gate: before anything is overwritten, the operator downloads root's certificate
//! from a one-time HTTPS page served with the *new* bia's server certificate. The page
//! shows the CA fingerprint the terminal shows too. After the takeover the only way in
//! is that certificate, so the takeover waits until it was downloaded (or copied).
use anyhow::{Context, Result};
use bia_pki::Pki;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct Gate {
    pub port: u16,
    pub token: String,
    pub downloaded: Arc<AtomicBool>,
}

/// Serve `p12` (and the CA) at `https://<any address>:<port>/<token>/` until the process ends.
pub fn start(files: &std::path::Path, port: u16, p12: Vec<u8>, ca_sha256: String) -> Result<Gate> {
    bia_pki::install_crypto_provider();
    let pki = Pki::load_or_create(files)?;
    let cfg = pki.server_config()?;
    let ca_pem = pki.ca_pem()?;
    let listener = TcpListener::bind(("0.0.0.0", port))
        .or_else(|_| TcpListener::bind(("::", port)))
        .with_context(|| format!("port {port} is taken; pass --gate-port"))?;
    let port = listener.local_addr()?.port();
    let mut t = [0u8; 16];
    getrandom::getrandom(&mut t).ok();
    let token: String = t.iter().map(|b| format!("{b:02x}")).collect();
    let downloaded = Arc::new(AtomicBool::new(false));
    let (tok, done) = (token.clone(), downloaded.clone());
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (cfg, tok, done, p12, ca_pem, ca_sha) =
                (cfg.clone(), tok.clone(), done.clone(), p12.clone(), ca_pem.clone(), ca_sha256.clone());
            std::thread::spawn(move || {
                let mut tcp = tcp;
                let _ = tcp.set_read_timeout(Some(std::time::Duration::from_secs(30)));
                let Ok(mut conn) = rustls::ServerConnection::new(cfg) else { return };
                while conn.is_handshaking() {
                    if conn.complete_io(&mut tcp).is_err() {
                        return;
                    }
                }
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let mut line = String::new();
                {
                    let mut rd = BufReader::new(&mut tls);
                    if rd.read_line(&mut line).is_err() {
                        return;
                    }
                    loop {
                        let mut h = String::new();
                        if rd.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let base = format!("/{tok}");
                let (code, ctype, extra, body): (u16, &str, String, Vec<u8>) = if path == base || path == format!("{base}/") {
                    (200, "text/html; charset=utf-8", String::new(), page(&tok, &ca_sha).into_bytes())
                } else if path == format!("{base}/root.p12") {
                    done.store(true, Ordering::SeqCst);
                    (200, "application/x-pkcs12", "Content-Disposition: attachment; filename=\"bia-root.p12\"\r\n".into(), p12)
                } else if path == format!("{base}/ca.crt") {
                    (200, "application/x-pem-file", "Content-Disposition: attachment; filename=\"bia-ca.crt\"\r\n".into(), ca_pem.into_bytes())
                } else {
                    (404, "text/plain", String::new(), b"not found".to_vec())
                };
                let _ = write!(
                    tls,
                    "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\n{extra}Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tls.write_all(&body);
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    Ok(Gate { port, token, downloaded })
}

fn page(tok: &str, ca_sha: &str) -> String {
    let fp: Vec<String> = ca_sha.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect();
    format!(
        r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>bia takeover</title>
<style>body{{font:16px/1.5 system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;color:#222}}code{{background:#f2f2f2;padding:.1rem .3rem;border-radius:4px;word-break:break-all}}a.b{{display:inline-block;margin:.3rem .5rem .3rem 0;padding:.6rem 1rem;background:#2d6cdf;color:#fff;border-radius:8px;text-decoration:none}}</style>
<h1>Your key to bia</h1>
<p>This machine is about to become a bia. Afterwards the only way in is its web app on HTTPS, with this certificate.</p>
<p><a class="b" href="/{tok}/root.p12">Download bia-root.p12</a><a class="b" href="/{tok}/ca.crt">Download the CA</a></p>
<p>Import <b>bia-root.p12</b> into your browser or system (password <code>12345678</code>), and trust the CA if you like.
Its fingerprint (SHA-256) must match what the terminal shows:</p>
<p><code>{}</code></p>
<p>Then go back to the terminal and confirm.</p>
"#,
        fp.join(" ")
    )
}
