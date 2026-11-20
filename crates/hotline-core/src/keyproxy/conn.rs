//! One job's connection: a blind tunnel, or a terminated bound origin.

use std::convert::Infallible;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, Request, Response, StatusCode, Version};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::client::conn::http1::SendRequest;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::scrub::{Pairs, ScrubBody, Scrubber};
use super::table::{origin, parse_authority};
use super::{Address, Inner};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Reply = Response<BoxBody<Bytes, BoxError>>;

const MAX_HEAD_BYTES: usize = 16 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
/// Request headers that belong to one hop, or that the proxy sets itself.
const HOP: [&str; 11] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "expect",
    "host",
    "accept-encoding",
];
const REPLY_HOP: [&str; 6] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "trailer",
];

pub(super) async fn serve(inner: Arc<Inner>, listener: TcpListener) -> io::Result<()> {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let inner = Arc::clone(&inner);
                tokio::spawn(async move {
                    let _ = handle(inner, stream).await;
                });
            }
            Err(error) => {
                eprintln!("hotline-computer: key proxy: accept: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle(inner: Arc<Inner>, stream: TcpStream) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    let mut client = BufReader::new(stream);
    let Ok(Ok(head)) = timeout(HEAD_TIMEOUT, read_head(&mut client)).await else {
        return Ok(());
    };
    let (host, port) = match connect_target(&head) {
        Ok(target) => target,
        Err((status, text)) => {
            let reply = format!(
                "HTTP/1.1 {status}\r\nAllow: CONNECT\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            return client.write_all(reply.as_bytes()).await;
        }
    };
    let origin = origin(&host, port);
    // The run this connection is for, and whether it terminates here.
    let admitted = {
        let table = inner
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        proxy_authorization(&head)
            .and_then(|header| table.authorized(header))
            .map(|run| {
                let bound = table.is_bound(&run, &origin);
                (run, bound)
            })
    };
    let Some((run, bound)) = admitted else {
        let text = "this proxy is a run's own: use the HTTPS_PROXY it was given\n";
        let reply = format!(
            "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"Hotline\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        );
        return client.write_all(reply.as_bytes()).await;
    };
    if bound {
        terminate(inner, client, host, port, origin, run).await
    } else {
        tunnel(inner, client, &host, port).await
    }
}

async fn read_head(client: &mut BufReader<TcpStream>) -> io::Result<String> {
    let mut head = Vec::new();
    loop {
        let before = head.len();
        let read = client.read_until(b'\n', &mut head).await?;
        if read == 0 || head.len() > MAX_HEAD_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if matches!(&head[before..], b"\r\n" | b"\n") {
            return String::from_utf8(head).map_err(|_| io::ErrorKind::InvalidData.into());
        }
    }
}

/// The `Proxy-Authorization` header of a CONNECT's head, if it has one.
fn proxy_authorization(head: &str) -> Option<&str> {
    head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("proxy-authorization")
            .then(|| value.trim())
    })
}

fn connect_target(head: &str) -> Result<(String, u16), (&'static str, &'static str)> {
    let line = head.lines().next().unwrap_or_default();
    let mut words = line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version)) = (words.next(), words.next(), words.next())
    else {
        return Err(("400 Bad Request", "not an HTTP request\n"));
    };
    if method != "CONNECT" {
        return Err((
            "405 Method Not Allowed",
            "this proxy takes CONNECT only: use it as HTTPS_PROXY\n",
        ));
    }
    if !version.starts_with("HTTP/1.") {
        return Err(("400 Bad Request", "HTTP/1 only\n"));
    }
    parse_authority(target, 443).ok_or(("400 Bad Request", "CONNECT names no host:port\n"))
}

async fn open(inner: &Inner, host: &str, port: u16) -> io::Result<TcpStream> {
    let connect = async {
        match inner.upstream.address(host, port) {
            Address::Name(host, port) => TcpStream::connect((host.as_str(), port)).await,
            Address::Fixed(address) => TcpStream::connect(address).await,
        }
    };
    let stream = timeout(CONNECT_TIMEOUT, connect)
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// Every host no variable is bound to: bytes both ways, never decrypted.
async fn tunnel(
    inner: Arc<Inner>,
    mut client: BufReader<TcpStream>,
    host: &str,
    port: u16,
) -> io::Result<()> {
    let mut upstream = match open(&inner, host, port).await {
        Ok(upstream) => upstream,
        Err(error) => {
            let text = format!("cannot reach {host}:{port}: {error}\n");
            let reply = format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            return client.write_all(reply.as_bytes()).await;
        }
    };
    client.write_all(ESTABLISHED).await?;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

/// A bound origin: TLS ends here, each request is checked and forwarded.
async fn terminate(
    inner: Arc<Inner>,
    mut client: BufReader<TcpStream>,
    host: String,
    port: u16,
    origin: String,
    run: String,
) -> io::Result<()> {
    let config = inner.ca.server_config(&host).map_err(io::Error::other)?;
    client.write_all(ESTABLISHED).await?;
    let tls = TlsAcceptor::from(config).accept(client).await?;
    let session = Arc::new(Session {
        inner,
        host,
        port,
        origin,
        run,
        upstream: AsyncMutex::new(None),
    });
    let service = service_fn(move |request| {
        let session = Arc::clone(&session);
        async move { Ok::<_, Infallible>(session.forward(request).await) }
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(tls), service)
        .await;
    Ok(())
}

struct Session {
    inner: Arc<Inner>,
    host: String,
    port: u16,
    origin: String,
    /// The proxy user that opened this connection: only its stand-ins are
    /// swapped here.
    run: String,
    /// One upstream connection, reused while it stays open.
    upstream: AsyncMutex<Option<SendRequest<Incoming>>>,
}

impl Session {
    async fn forward(&self, request: Request<Incoming>) -> Reply {
        let (parts, body) = request.into_parts();
        if !self.same_place(&parts.uri, &parts.headers) {
            return plain(
                StatusCode::BAD_REQUEST,
                "the request names a different host than its CONNECT\n",
            );
        }
        let mut headers = forwarded_headers(&parts.headers);
        let authority = self
            .origin
            .strip_prefix("https://")
            .unwrap_or(&self.origin)
            .to_owned();
        headers.insert(
            header::HOST,
            HeaderValue::from_str(&authority).expect("an origin is a header value"),
        );
        // A reply the proxy can read is one it can scrub.
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        let (names, scrub) = self.swap(&mut headers);
        let swapped = !names.is_empty();
        if swapped
            && self
                .table()
                .refused(&self.run, &names, &self.origin, parts.uri.path())
        {
            return plain(
                StatusCode::FORBIDDEN,
                "Hotline does not send this key to a route that makes new keys\n",
            );
        }
        let target = parts
            .uri
            .path_and_query()
            .map_or("/", http::uri::PathAndQuery::as_str);
        let mut upstream = Request::new(body);
        *upstream.method_mut() = parts.method;
        *upstream.version_mut() = Version::HTTP_11;
        *upstream.headers_mut() = headers;
        match target.parse() {
            Ok(uri) => *upstream.uri_mut() = uri,
            Err(_) => return plain(StatusCode::BAD_REQUEST, "bad request target\n"),
        }
        let response = match self.send(upstream).await {
            Ok(response) => response,
            Err(error) => return plain(StatusCode::BAD_GATEWAY, &format!("{error}\n")),
        };
        let (mut parts, body) = response.into_parts();
        let encoded = parts
            .headers
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"));
        if swapped && encoded {
            return plain(
                StatusCode::BAD_GATEWAY,
                "the service answered in an encoding the proxy cannot check\n",
            );
        }
        let scrubber = Scrubber::new(scrub);
        let mut scrubbed = HeaderMap::with_capacity(parts.headers.len());
        for (name, value) in &parts.headers {
            if REPLY_HOP.contains(&name.as_str()) {
                continue;
            }
            if let Ok(value) = HeaderValue::from_bytes(&scrubber.all(value.as_bytes())) {
                scrubbed.append(name.clone(), value);
            }
        }
        if !scrubber.keeps_length() {
            scrubbed.remove(header::CONTENT_LENGTH);
        }
        parts.headers = scrubbed;
        let body = ScrubBody::new(body, scrubber)
            .map_err(|error| Box::new(error) as BoxError)
            .boxed();
        Response::from_parts(parts, body)
    }

    /// Whether the request's own host, if it names one, is the CONNECT's.
    fn same_place(&self, uri: &http::Uri, headers: &HeaderMap) -> bool {
        let matches = |text: &str| {
            !text.contains('@')
                && parse_authority(text, 443).is_some_and(|(h, p)| h == self.host && p == self.port)
        };
        if let Some(authority) = uri.authority()
            && !matches(authority.as_str())
        {
            return false;
        }
        let mut hosts = headers.get_all(header::HOST).iter();
        match (hosts.next(), hosts.next()) {
            (None, _) => true,
            (Some(host), None) => host.to_str().is_ok_and(matches),
            (Some(_), Some(_)) => false,
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, super::table::Table> {
        self.inner
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Puts each live stand-in's value in the bound headers. Returns the
    /// variables that went in, and what to write back over in the reply.
    fn swap(&self, headers: &mut HeaderMap) -> (Vec<String>, Pairs) {
        let table = self.table();
        let mut scrub = Vec::new();
        let mut names = Vec::new();
        for name in table.headers(&self.run, &self.origin) {
            let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
                continue;
            };
            let values: Vec<HeaderValue> = headers.get_all(&name).iter().cloned().collect();
            if values.is_empty() {
                continue;
            }
            headers.remove(&name);
            for value in values {
                let swap = value
                    .to_str()
                    .ok()
                    .and_then(|text| table.swap(&self.run, &self.origin, name.as_str(), text));
                let Some(swap) = swap else {
                    headers.append(name.clone(), value);
                    continue;
                };
                let Ok(mut sent) = HeaderValue::from_str(&swap.header) else {
                    headers.append(name.clone(), value);
                    continue;
                };
                sent.set_sensitive(true);
                headers.append(name.clone(), sent);
                scrub.extend(swap.scrub);
                names.extend(swap.names);
            }
        }
        scrub.extend(table.redactions(&self.run));
        (names, scrub)
    }

    async fn send(&self, request: Request<Incoming>) -> Result<Response<Incoming>, String> {
        let mut upstream = self.upstream.lock().await;
        if upstream.as_ref().is_none_or(SendRequest::is_closed) {
            *upstream = Some(self.connect().await?);
        }
        let sender = upstream.as_mut().expect("just connected");
        sender
            .ready()
            .await
            .map_err(|error| format!("{} closed: {error}", self.origin))?;
        sender
            .send_request(request)
            .await
            .map_err(|error| format!("{}: {error}", self.origin))
    }

    /// The upstream, which must prove the origin's name against the
    /// compiled-in roots before anything is sent to it.
    async fn connect(&self) -> Result<SendRequest<Incoming>, String> {
        let tcp = open(&self.inner, &self.host, self.port)
            .await
            .map_err(|error| format!("cannot reach {}: {error}", self.origin))?;
        let name = ServerName::try_from(self.host.clone())
            .map_err(|_| format!("{} is not a server name", self.host))?;
        let tls = TlsConnector::from(Arc::clone(&self.inner.upstream.tls))
            .connect(name, tcp)
            .await
            .map_err(|error| format!("{} is not who it says: {error}", self.origin))?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|error| format!("{}: {error}", self.origin))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(sender)
    }
}

fn forwarded_headers(from: &HeaderMap) -> HeaderMap {
    let named: Vec<String> = from
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|v| v.trim().to_ascii_lowercase())
        .collect();
    let mut headers = HeaderMap::with_capacity(from.len());
    for (name, value) in from {
        if HOP.contains(&name.as_str()) || named.iter().any(|n| n == name.as_str()) {
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    headers
}

fn plain(status: StatusCode, text: &str) -> Reply {
    let mut reply = Response::new(
        Full::new(Bytes::from(text.to_owned()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *reply.status_mut() = status;
    reply.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    reply
}
