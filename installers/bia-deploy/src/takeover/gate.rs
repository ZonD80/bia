//! The gate: before anything is overwritten, the operator downloads root's certificate
//! from a one-time HTTPS page served with the *new* bia's server certificate: as a .p12 for a
//! browser, or inside a .biaconfig for the bia deploy app, encrypted with a password the
//! operator sends. The page shows the CA fingerprint the terminal shows too. After the takeover the only way in
//! is that certificate, so the takeover waits until it was downloaded.
use anyhow::{Context, Result};
use bia_pki::biaconfig::{self, BiaConfig};
use bia_pki::Pki;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct Gate {
    pub port: u16,
    pub token: String,
    /// One of the two files went out whole: root's key has left this machine.
    pub downloaded: Arc<AtomicBool>,
    /// Both files went out.
    pub both: Arc<AtomicBool>,
    /// The page was opened: a person may still want the other file.
    pub browsed: Arc<AtomicBool>,
}

/// What a `.biaconfig` made at the gate says besides the certificate.
#[derive(Clone)]
pub struct Reach {
    /// Where the new bia's web app will answer, the best first; the server certificate names all of them.
    pub addresses: Vec<String>,
    pub ca_pem: String,
}

enum Sent {
    Other,
    P12,
    Config,
}

/// Serve `p12` (by itself, and inside a `.biaconfig`) at `https://<any address>:<port>/<token>/`
/// until the process ends.
pub fn start(files: &std::path::Path, port: u16, p12: Vec<u8>, ca_sha256: String, reach: Reach) -> Result<Gate> {
    bia_pki::install_crypto_provider();
    let pki = Pki::load_or_create(files)?;
    let cfg = pki.server_config()?;
    let ca_name = pki.ca_name();
    // what root called this bia in the takeover's settings, if anything
    let bia_name = pki.chosen_name().unwrap_or_default();
    let listener = TcpListener::bind(("0.0.0.0", port))
        .or_else(|_| TcpListener::bind(("::", port)))
        .with_context(|| format!("port {port} is taken; pass --gate-port"))?;
    let port = listener.local_addr()?.port();
    let mut t = [0u8; 16];
    getrandom::getrandom(&mut t).ok();
    let token: String = t.iter().map(|b| format!("{b:02x}")).collect();
    let downloaded = Arc::new(AtomicBool::new(false));
    let both = Arc::new(AtomicBool::new(false));
    let browsed = Arc::new(AtomicBool::new(false));
    let got = Arc::new((AtomicBool::new(false), AtomicBool::new(false)));
    let (tok, done, all, seen) = (token.clone(), downloaded.clone(), both.clone(), browsed.clone());
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (cfg, tok, done, all, seen, got, p12, ca_sha, ca_name, bia_name, reach) = (
                cfg.clone(),
                tok.clone(),
                done.clone(),
                all.clone(),
                seen.clone(),
                got.clone(),
                p12.clone(),
                ca_sha256.clone(),
                ca_name.clone(),
                bia_name.clone(),
                reach.clone(),
            );
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
                let (mut host, mut body) = (String::new(), Vec::new());
                {
                    let mut rd = BufReader::new(&mut tls);
                    if rd.read_line(&mut line).is_err() {
                        return;
                    }
                    let mut len = 0usize;
                    loop {
                        let mut h = String::new();
                        if rd.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':') {
                            match k.trim().to_ascii_lowercase().as_str() {
                                "host" => host = v.trim().to_string(),
                                "content-length" => len = v.trim().parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                    }
                    if len > 0 {
                        body = vec![0u8; len.min(MAX_FORM)];
                        if rd.read_exact(&mut body).is_err() {
                            return;
                        }
                    }
                }
                let mut words = line.split_whitespace();
                let method = words.next().unwrap_or("GET").to_string();
                let path = words.next().unwrap_or("/").to_string();
                let base = format!("/{tok}");
                let plain = |code: u16, text: String| (code, "text/plain; charset=utf-8", String::new(), text.into_bytes(), Sent::Other);
                let (code, ctype, extra, body, sent): (u16, &str, String, Vec<u8>, Sent) = if path == base || path == format!("{base}/") {
                    seen.store(true, Ordering::SeqCst);
                    let suggested = biaconfig::generate_password().unwrap_or_default();
                    (200, "text/html; charset=utf-8", String::new(), page(&tok, &ca_sha, &ca_name, &suggested).into_bytes(), Sent::Other)
                } else if path == format!("{base}/root.p12") {
                    (200, "application/x-pkcs12", "Content-Disposition: attachment; filename=\"bia-root.p12\"\r\n".into(), p12, Sent::P12)
                } else if path == format!("{base}/root.biaconfig") {
                    match form_field(&body, "password") {
                        _ if method != "POST" => plain(405, format!("POST a password of at least {} characters: curl -fk --data-urlencode 'password=…' -o bia.biaconfig <this address>\n", biaconfig::MIN_PASSWORD)),
                        None => plain(400, "no password was sent\n".into()),
                        Some(password) => {
                            let addresses = dialled_first(&reach.addresses, &host);
                            let name = biaconfig::file_name(addresses.first().map_or("", String::as_str));
                            let config = BiaConfig { name: bia_name, addresses, ca: reach.ca_pem.clone(), p12 };
                            match biaconfig::seal(&config, &password) {
                                Ok(file) => (200, biaconfig::MIME, format!("Content-Disposition: attachment; filename=\"{name}\"\r\n"), file, Sent::Config),
                                Err(e) => plain(400, format!("{e:#}\n")),
                            }
                        }
                    }
                } else {
                    plain(404, "not found".into())
                };
                let _ = write!(
                    tls,
                    "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\n{extra}Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let ok = tls.write_all(&body).and_then(|_| tls.flush()).is_ok();
                tls.conn.send_close_notify();
                let _ = tls.flush();
                // downloaded = the whole key went out, not just asked for
                match sent {
                    Sent::P12 if ok => got.0.store(true, Ordering::SeqCst),
                    Sent::Config if ok => got.1.store(true, Ordering::SeqCst),
                    _ => return,
                }
                all.store(got.0.load(Ordering::SeqCst) && got.1.load(Ordering::SeqCst), Ordering::SeqCst);
                done.store(true, Ordering::SeqCst);
            });
        }
    });
    Ok(Gate { port, token, downloaded, both, browsed })
}

/// A form is one password.
const MAX_FORM: usize = 4096;

/// One field of an `application/x-www-form-urlencoded` body.
fn form_field(body: &[u8], key: &str) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    text.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| unescape(v))
    })
}

fn unescape(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |j: usize| b.get(j).and_then(|c| (*c as char).to_digit(16));
        match (b[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 2;
            }
            (b'+', ..) => out.push(b' '),
            (c, ..) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The address this very request reached the machine at goes first, when the server
/// certificate names it: that one is known to work from where the person is.
fn dialled_first(addresses: &[String], host_header: &str) -> Vec<String> {
    let host = match host_header.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host_header.rsplit_once(':').map_or(host_header, |(h, _)| h),
    };
    let mut out = addresses.to_vec();
    if let Some(i) = out.iter().position(|a| a.eq_ignore_ascii_case(host)) {
        let first = out.remove(i);
        out.insert(0, first);
    }
    out
}

fn page(tok: &str, ca_sha: &str, ca_name: &str, suggested: &str) -> String {
    let fp: Vec<String> = ca_sha.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect();
    let min = biaconfig::MIN_PASSWORD;
    format!(
        r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>bia takeover</title>
<style>body{{font:16px/1.5 system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;color:#222}}code{{background:#f2f2f2;padding:.1rem .3rem;border-radius:4px;word-break:break-all}}a.b,button.b{{display:inline-block;margin:.3rem .5rem .3rem 0;padding:.6rem 1rem;background:#2d6cdf;color:#fff;border:0;border-radius:8px;font:inherit;text-decoration:none;cursor:pointer}}input{{font:inherit;font-family:ui-monospace,monospace;padding:.5rem;border:1px solid #bbb;border-radius:8px;width:15rem;max-width:100%;box-sizing:border-box}}</style>
<h1>Your key to bia</h1>
<p>This machine is about to become a bia. Afterwards the only way in is its web app on HTTPS, with this certificate.
It comes as two files; one is enough, and the takeover continues once a file was downloaded.
To have both, download one right after the other.</p>
<h2>For a browser</h2>
<p><a class="b" href="/{tok}/root.p12">Download bia-root.p12</a></p>
<p>Import <b>bia-root.p12</b> into your browser or system (password <code>12345678</code>). It also holds bia's
certificate authority, <b>{ca_name}</b>; trust it so your browser knows bia without warnings.</p>
<h2>For the bia deploy app</h2>
<p>A <b>.biaconfig</b> holds the same certificate and this server's address, encrypted with the password below
(at least {min} characters; change it if you like). Write the password down: the app on your phone asks for it when it opens the file.</p>
<form method="post" action="/{tok}/root.biaconfig">
<input name="password" value="{suggested}" minlength="{min}" required autocomplete="off" autocapitalize="off" spellcheck="false" aria-label="Password for the .biaconfig">
<button class="b">Download .biaconfig</button>
</form>
<h2>Check the fingerprint</h2>
<p>The certificate authority's fingerprint (SHA-256) must match what the terminal shows:</p>
<p><code>{}</code></p>
<p>The terminal goes on by itself after the download.</p>
"#,
        fp.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One HTTPS request to the gate, trusting only the new bia's CA.
    fn ask(port: u16, ca_pem: &str, request: &str) -> (String, Vec<u8>) {
        let mut roots = rustls::RootCertStore::empty();
        for c in bia_pki::pem_to_cert_ders(ca_pem).unwrap() {
            roots.add(c).unwrap();
        }
        let cfg = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let conn = rustls::ClientConnection::new(Arc::new(cfg), "bia".try_into().unwrap()).unwrap();
        let tcp = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        tls.write_all(request.as_bytes()).unwrap();
        let mut all = Vec::new();
        let _ = tls.read_to_end(&mut all);
        let cut = all.windows(4).position(|w| w == b"\r\n\r\n").expect("a full reply");
        (String::from_utf8_lossy(&all[..cut]).into_owned(), all[cut + 4..].to_vec())
    }

    #[test]
    fn the_gate_hands_out_a_biaconfig_that_opens_to_the_same_key() {
        let files = std::env::temp_dir().join(format!("bia-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&files);
        std::fs::create_dir_all(&files).unwrap();
        let ip: std::net::IpAddr = "203.0.113.7".parse().unwrap();
        std::fs::create_dir_all(files.join("state")).unwrap();
        std::fs::write(files.join("state/self.json"), br#"{"name":"Fred"}"#).unwrap();
        let seeded = bia_pki::seed(
            &files,
            &bia_pki::SeedOptions { names: vec!["bia.example".into()], ips: vec![ip], root: bia_pki::RootSeed::Generate, root_from: vec![], agent_name: None },
        )
        .unwrap();
        let p12 = seeded.root_p12.clone().unwrap();
        let ca_pem = std::fs::read_to_string(seeded.dir.join("ca.crt")).unwrap();
        let reach = Reach { addresses: vec!["bia.example".into(), ip.to_string()], ca_pem: ca_pem.clone() };
        let g = start(&files, 0, p12.clone(), seeded.ca_sha256.clone(), reach).unwrap();
        let at = format!("/{}/root.biaconfig", g.token);
        let post = |form: &str| ask(g.port, &ca_pem, &format!("POST {at} HTTP/1.1\r\nHost: 203.0.113.7:{}\r\nContent-Length: {}\r\n\r\n{form}", g.port, form.len()));

        // the page offers both files and a password nobody had to invent
        let (head, body) = ask(g.port, &ca_pem, &format!("GET /{}/ HTTP/1.1\r\nHost: x\r\n\r\n", g.token));
        let html = String::from_utf8_lossy(&body).into_owned();
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(html.contains("root.p12") && html.contains("root.biaconfig"), "{html}");
        assert!(g.browsed.load(Ordering::SeqCst));

        // refused: nothing counts as downloaded
        assert!(post("password=short").0.starts_with("HTTP/1.1 400"));
        assert!(post("other=1").0.starts_with("HTTP/1.1 400"));
        assert!(ask(g.port, &ca_pem, &format!("GET {at} HTTP/1.1\r\nHost: x\r\n\r\n")).0.starts_with("HTTP/1.1 405"));
        assert!(ask(g.port, &ca_pem, "POST /wrong/root.biaconfig HTTP/1.1\r\nHost: x\r\n\r\n").0.starts_with("HTTP/1.1 404"));
        assert!(!g.downloaded.load(Ordering::SeqCst));

        let (head, file) = post("password=correct+horse%21");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("filename=\"bia-203.0.113.7.biaconfig\""), "{head}");
        assert!(biaconfig::is_biaconfig(&file));
        assert!(biaconfig::open(&file, "wrong password").is_err());
        let c = biaconfig::open(&file, "correct horse!").unwrap();
        assert_eq!(c.p12, p12);
        assert_eq!(c.ca, ca_pem);
        assert_eq!(c.name, "Fred");
        // the address the request came to is first
        assert_eq!(c.addresses, vec!["203.0.113.7".to_string(), "bia.example".to_string()]);
        assert_eq!(biaconfig::details(&c).unwrap().identity, "root");
        assert!(g.downloaded.load(Ordering::SeqCst) && !g.both.load(Ordering::SeqCst));

        let (_, got) = ask(g.port, &ca_pem, &format!("GET /{}/root.p12 HTTP/1.1\r\nHost: x\r\n\r\n", g.token));
        assert_eq!(got, p12);
        assert!(g.both.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&files);
    }

    #[test]
    fn a_form_password_is_decoded() {
        assert_eq!(form_field(b"a=1&password=p%40ss+word%2B1", "password").as_deref(), Some("p@ss word+1"));
        assert_eq!(form_field(b"a=1", "password"), None);
        assert_eq!(form_field(b"password=100%", "password").as_deref(), Some("100%"));
    }

    #[test]
    fn the_dialled_address_goes_first_only_if_the_certificate_names_it() {
        let a = vec!["bia.example".to_string(), "10.0.0.5".to_string(), "2001:db8::1".to_string()];
        assert_eq!(dialled_first(&a, "10.0.0.5:8443")[0], "10.0.0.5");
        assert_eq!(dialled_first(&a, "[2001:db8::1]:8443")[0], "2001:db8::1");
        assert_eq!(dialled_first(&a, "BIA.example:8443")[0], "bia.example");
        assert_eq!(dialled_first(&a, "127.0.0.1:8443"), a);
        assert_eq!(dialled_first(&a, ""), a);
    }
}
