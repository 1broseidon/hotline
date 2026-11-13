//! Which addresses an MCP server's sign-in may connect to, and the HTTP
//! client that holds every sign-in request to it.
//!
//! A server names, in the metadata it publishes, the hosts its sign-in is
//! done at, and the desk sends them requests. A server on the internet must
//! not be able to aim those at this computer or the network it is on
//! (RUSTSEC-2026-0457), while a server the person runs on that network has to
//! reach its neighbours. So the server's own address decides, once, when the
//! client is made:
//!
//! - **On the internet** (a public IP address, a name that answers any
//!   public address, or a name that answers nothing): public addresses only.
//!   Of a name that answers both kinds only the public ones are connected to.
//! - **On this computer or network** (a loopback or private IP address, or a
//!   name that answers no public address and at least one of those): private
//!   addresses too, and loopback ones when the server is itself on loopback.
//!   A name is then held to the addresses it answered: this client never
//!   asks for it again, so a name that answered a private address to pass
//!   the check cannot answer the internet's afterwards and have what is
//!   served from there trusted with the network (DNS rebinding).
//!
//! Link-local addresses (cloud instance metadata lives there), the
//! unspecified, broadcast, multicast and reserved ones are never connected
//! to, bar the server's own when its URL is that IP address.
//!
//! The rule is checked where the connection is made, not before it: this
//! client's name lookups go through `Reach`, which hands the connector only
//! the addresses it admits, and a URL whose host is an IP address, which is
//! connected to without a lookup, is checked before each request and at each
//! redirect. The classes of address are rmcp's own (`AddressClass` in its
//! `transport/auth/metadata_resolver.rs`), which its type keeps private, so
//! they are repeated here and move when rmcp's do.
//!
//! Two limits. rmcp also refuses, by the host's spelling alone, a metadata
//! document or authorization server named by a loopback or private IP
//! address unless both it and the server are on loopback; only its own
//! client can relax that, so such a server is named by host name instead.
//! And a sign-in on the internet goes through a configured proxy as every
//! other request of the desk's does: the proxy looks the names up, so only
//! an IP address in a URL is checked there, as with rmcp's own client.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRedirectPolicy,
    OAuthHttpRequest,
};
use std::error::Error;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use url::{Host, Url};

/// What the person reads when a server on the internet is refused.
pub(super) const REFUSED_FROM_THE_INTERNET: &str = "MCP OAuth sign-in refused: this server is on the internet but points to a private network address.";
/// What they read when discovery fails for any other reason.
pub(super) const DISCOVERY_FAILED: &str = "MCP OAuth metadata discovery failed.";

/// What answers for DNS: the system's resolver on a desk, a table in a test.
pub type NameLookup = Arc<dyn Resolve>;

/// rmcp's own bounds for a sign-in request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 10;
/// How long the server's own name may take to answer when the client is
/// made, outside any request's timeout. A name that does not answer is taken
/// for one on the internet, and its discovery then fails by itself.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The variables reqwest reads a proxy from.
const PROXY_VARIABLES: [&str; 6] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];

pub(super) fn system_lookup() -> NameLookup {
    Arc::new(SystemLookup)
}

struct SystemLookup;

impl Resolve for SystemLookup {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddressClass {
    Public,
    Loopback,
    /// RFC 1918, carrier-grade NAT (a tailnet's `100.64.0.0/10`) and
    /// unique-local IPv6.
    Private,
    /// Link-local, unspecified, broadcast, multicast and reserved.
    Blocked,
}

impl AddressClass {
    fn of(address: IpAddr) -> Self {
        match address {
            IpAddr::V4(v4) => Self::of_v4(v4),
            IpAddr::V6(v6) => match embedded_v4(v6) {
                Some(v4) => Self::of_v4(v4),
                None => Self::of_v6(v6),
            },
        }
    }

    fn of_v4(address: Ipv4Addr) -> Self {
        let [a, b, ..] = address.octets();
        let carrier_nat = a == 100 && (64..=127).contains(&b);
        if address.is_loopback() {
            Self::Loopback
        } else if address.is_private() || carrier_nat {
            Self::Private
        } else if address.is_link_local()
            || address.is_broadcast()
            || address.is_unspecified()
            || address.is_multicast()
            || a == 0
            || (a == 198 && matches!(b, 18 | 19))
        {
            Self::Blocked
        } else {
            Self::Public
        }
    }

    fn of_v6(address: Ipv6Addr) -> Self {
        let segments = address.segments();
        if address.is_loopback() {
            Self::Loopback
        } else if (segments[0] & 0xfe00) == 0xfc00 {
            Self::Private
        } else if address.is_unspecified()
            || address.is_multicast()
            || (segments[0] & 0xffc0) == 0xfe80
            // Teredo and local-use NAT64 carry an IPv4 address this does
            // not read, so they are refused whole.
            || segments[..2] == [0x2001, 0]
            || segments[..3] == [0x64, 0xff9b, 1]
        {
            Self::Blocked
        } else {
            Self::Public
        }
    }
}

/// The IPv4 address an IPv6 one carries: IPv4-mapped, IPv4-compatible,
/// NAT64 (`64:ff9b::/96`) and 6to4 (`2002::/16`). Such an address is judged
/// as the IPv4 one it reaches.
fn embedded_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let [a, b, c, d, e, f, g, h] = address.segments();
    let low = Ipv4Addr::from((u32::from(g) << 16) | u32::from(h));
    match [a, b, c, d, e, f] {
        [0, 0, 0, 0, 0, 0xffff] | [0x64, 0xff9b, 0, 0, 0, 0] => Some(low),
        [0, 0, 0, 0, 0, 0] if !address.is_loopback() && !address.is_unspecified() => Some(low),
        [0x2002, ..] => Some(Ipv4Addr::from((u32::from(b) << 16) | u32::from(c))),
        _ => None,
    }
}

/// A request sign-in did not make, because of the address it was for.
#[derive(Clone, Debug)]
pub(super) struct Refused {
    host: String,
    /// Whether the server is on the internet, which is when the person is
    /// told in the words above.
    pub(super) from_the_internet: bool,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.from_the_internet {
            // rmcp keeps only the text of some errors, so the text is the
            // sentence the person reads.
            f.write_str(REFUSED_FROM_THE_INTERNET)
        } else {
            write!(
                f,
                "no address of `{}` is allowed for this sign-in",
                self.host
            )
        }
    }
}

impl Error for Refused {}

impl Refused {
    /// What the person reads for it.
    pub(super) fn message(&self) -> &'static str {
        if self.from_the_internet {
            REFUSED_FROM_THE_INTERNET
        } else {
            DISCOVERY_FAILED
        }
    }
}

/// The refusal an error was caused by, wherever in its chain.
pub(super) fn refusal<'a>(error: &'a (dyn Error + 'static)) -> Option<&'a Refused> {
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(refused) = error.downcast_ref::<Refused>() {
            return Some(refused);
        }
        next = error.source();
    }
    None
}

/// What one server's sign-in may connect to.
struct Reach {
    loopback: bool,
    private: bool,
    /// The server's own address when its URL is an IP address: always
    /// reachable, whatever its class.
    literal: Option<IpAddr>,
    /// A server on this computer or network set up by name: the name, and
    /// the addresses it answered when it was checked.
    held: Option<(String, Vec<IpAddr>)>,
    /// The hosts of the proxies in the environment, whose own addresses are
    /// not judged: a request through one looks up only the proxy.
    proxies: Vec<String>,
    lookup: NameLookup,
}

impl Reach {
    async fn of(server: &Url, lookup: NameLookup) -> Self {
        let mut reach = Self {
            loopback: false,
            private: false,
            literal: None,
            held: None,
            proxies: Vec::new(),
            lookup,
        };
        let classes = match server.host() {
            Some(Host::Ipv4(v4)) => {
                reach.literal = Some(IpAddr::V4(v4));
                vec![AddressClass::of(IpAddr::V4(v4))]
            }
            Some(Host::Ipv6(v6)) => {
                reach.literal = Some(IpAddr::V6(v6));
                vec![AddressClass::of(IpAddr::V6(v6))]
            }
            Some(Host::Domain(domain)) => {
                let answered = reach.answers(domain).await;
                let classes: Vec<AddressClass> =
                    answered.iter().map(|ip| AddressClass::of(*ip)).collect();
                let local: Vec<IpAddr> = answered
                    .into_iter()
                    .filter(|ip| {
                        matches!(
                            AddressClass::of(*ip),
                            AddressClass::Loopback | AddressClass::Private
                        )
                    })
                    .collect();
                if classes.contains(&AddressClass::Public) || local.is_empty() {
                    Vec::new()
                } else {
                    reach.held = Some((normalize(domain), local));
                    classes
                }
            }
            None => Vec::new(),
        };
        reach.loopback = classes.contains(&AddressClass::Loopback);
        reach.private = reach.loopback || classes.contains(&AddressClass::Private);
        if reach.on_the_internet() {
            reach.proxies = proxy_hosts(
                PROXY_VARIABLES
                    .iter()
                    .filter_map(|variable| std::env::var(variable).ok()),
            );
        }
        reach
    }

    /// Every address the server's own name answers now, or none.
    async fn answers(&self, domain: &str) -> Vec<IpAddr> {
        let Ok(name) = domain.parse::<Name>() else {
            return Vec::new();
        };
        match tokio::time::timeout(LOOKUP_TIMEOUT, self.lookup.resolve(name)).await {
            Ok(Ok(addresses)) => addresses.map(|address| address.ip()).collect(),
            _ => Vec::new(),
        }
    }

    fn on_the_internet(&self) -> bool {
        !self.private
    }

    fn admits(&self, address: IpAddr) -> bool {
        match AddressClass::of(address) {
            AddressClass::Public => true,
            AddressClass::Loopback => self.loopback,
            AddressClass::Private => self.private,
            AddressClass::Blocked => false,
        }
    }

    fn refused(&self, host: &str) -> Refused {
        Refused {
            host: host.to_string(),
            from_the_internet: self.on_the_internet(),
        }
    }

    /// A URL whose host is an IP address is connected to without a lookup,
    /// so it is judged here. A name is judged when it is looked up.
    fn check_literal(&self, url: &Url) -> Result<(), Refused> {
        let address = match url.host() {
            Some(Host::Ipv4(v4)) => IpAddr::V4(v4),
            Some(Host::Ipv6(v6)) => IpAddr::V6(v6),
            _ => return Ok(()),
        };
        if self.literal == Some(address) || self.admits(address) {
            Ok(())
        } else {
            Err(self.refused(&address.to_string()))
        }
    }

    /// The addresses of `name` a connection may be made to.
    async fn addresses(&self, name: Name) -> Result<Vec<SocketAddr>, OAuthHttpClientError> {
        let host = normalize(name.as_str());
        if let Some((held, addresses)) = &self.held
            && *held == host
        {
            return Ok(addresses.iter().map(|ip| SocketAddr::new(*ip, 0)).collect());
        }
        let answered: Vec<SocketAddr> = self.lookup.resolve(name).await?.collect();
        if self.proxies.contains(&host) {
            return Ok(answered);
        }
        let admitted: Vec<SocketAddr> = answered
            .into_iter()
            .filter(|address| self.admits(address.ip()))
            .collect();
        if admitted.is_empty() {
            return Err(Box::new(self.refused(&host)));
        }
        Ok(admitted)
    }
}

/// The lookup a sign-in's connections are made through.
struct Admitted(Arc<Reach>);

impl Resolve for Admitted {
    fn resolve(&self, name: Name) -> Resolving {
        let reach = self.0.clone();
        Box::pin(async move {
            let addresses = reach.addresses(name).await?;
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

fn normalize(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn proxy_hosts(proxies: impl Iterator<Item = String>) -> Vec<String> {
    proxies
        .filter_map(|proxy| {
            let proxy = if proxy.contains("://") {
                proxy
            } else {
                format!("http://{proxy}")
            };
            Url::parse(&proxy).ok()?.host_str().map(normalize)
        })
        .collect()
}

/// The client every request of one server's sign-in is made through:
/// discovery, registration, the token exchange and each refresh, by rmcp,
/// and the one discovery request this crate makes itself.
pub(super) struct SignInClient {
    reach: Arc<Reach>,
    following: reqwest::Client,
    stopping: reqwest::Client,
}

impl SignInClient {
    pub(super) async fn for_server(server: &Url, lookup: NameLookup) -> Result<Self, String> {
        let reach = Arc::new(Reach::of(server, lookup).await);
        let build = |redirects: reqwest::redirect::Policy| {
            let builder = reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .redirect(redirects)
                .dns_resolver(Admitted(reach.clone()));
            // A server on this computer or network is connected to directly:
            // a proxy would look the names up itself, past the addresses
            // its name is held to.
            let builder = if reach.on_the_internet() {
                builder
            } else {
                builder.no_proxy()
            };
            builder
                .build()
                .map_err(|_| "MCP OAuth HTTP client could not be created".to_string())
        };
        let at_redirect = reach.clone();
        let following = build(reqwest::redirect::Policy::custom(move |attempt| {
            if let Err(refused) = at_redirect.check_literal(attempt.url()) {
                attempt.error(refused)
            } else if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error(format!("more than {MAX_REDIRECTS} redirects"))
            } else {
                attempt.follow()
            }
        }))?;
        let stopping = build(reqwest::redirect::Policy::none())?;
        Ok(Self {
            reach,
            following,
            stopping,
        })
    }

    /// Why a request to one of `urls` would not be made now, for the first
    /// that would not: the question the connection asks, put before one is
    /// needed. The connection still asks it again.
    pub(super) async fn first_refused<'a>(
        &self,
        urls: impl IntoIterator<Item = &'a String>,
    ) -> Option<Refused> {
        for url in urls {
            let Ok(url) = Url::parse(url) else {
                continue;
            };
            if let Err(refused) = self.reach.check_literal(&url) {
                return Some(refused);
            }
            let Some(Host::Domain(domain)) = url.host() else {
                continue;
            };
            let Ok(name) = domain.parse::<Name>() else {
                continue;
            };
            if let Err(error) = self.reach.addresses(name).await
                && let Some(refused) = refusal(error.as_ref())
            {
                return Some(refused.clone());
            }
        }
        None
    }

    /// One GET with no redirect followed, for the server's own challenge.
    pub(super) async fn get(&self, url: &Url) -> Result<reqwest::Response, OAuthHttpClientError> {
        self.reach.check_literal(url)?;
        self.stopping
            .get(url.clone())
            .send()
            .await
            .map_err(refusal_first)
    }
}

/// A request the lookup or a redirect refused fails as reqwest's error with
/// the refusal beneath it. rmcp keeps only the top error's text for some
/// operations, so the refusal is put on top.
fn refusal_first(error: reqwest::Error) -> OAuthHttpClientError {
    match refusal(&error) {
        Some(refused) => Box::new(refused.clone()),
        None => Box::new(error),
    }
}

impl OAuthHttpClient for SignInClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let OAuthHttpRequest {
                request,
                redirect_policy,
                ..
            } = request;
            // A policy this was not written for is the stricter of the two.
            let client = match redirect_policy {
                OAuthHttpRedirectPolicy::Follow => &self.following,
                _ => &self.stopping,
            };
            let request = reqwest::Request::try_from(request)
                .map_err(|error| Box::new(error) as OAuthHttpClientError)?;
            self.reach.check_literal(request.url())?;
            let mut response = client.execute(request).await.map_err(refusal_first)?;

            let mut builder = http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(refusal_first)? {
                if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
                    return Err(format!(
                        "OAuth HTTP response body exceeds {MAX_RESPONSE_BYTES} bytes"
                    )
                    .into());
                }
                body.extend_from_slice(&chunk);
            }
            builder
                .body(body)
                .map_err(|error| Box::new(error) as OAuthHttpClientError)
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Answers each name with its listed answers in turn, then keeps giving
    /// the last; a name it was not given answers nothing. A name with the
    /// root's dot is the same name, as it is to DNS.
    struct Table(Mutex<HashMap<String, Vec<Vec<IpAddr>>>>);

    pub(in crate::oauth) fn table(names: &[(&str, &[&[&str]])]) -> NameLookup {
        let names = names
            .iter()
            .map(|(name, answers)| {
                let answers = answers
                    .iter()
                    .map(|answer| answer.iter().map(|ip| ip.parse().unwrap()).collect())
                    .collect();
                (name.to_string(), answers)
            })
            .collect();
        Arc::new(Table(Mutex::new(names)))
    }

    impl Resolve for Table {
        fn resolve(&self, name: Name) -> Resolving {
            let mut names = self.0.lock().unwrap();
            let answer = match names.get_mut(&normalize(name.as_str())) {
                Some(answers) if answers.len() > 1 => answers.remove(0),
                Some(answers) => answers[0].clone(),
                None => Vec::new(),
            };
            Box::pin(async move {
                let addresses = answer.into_iter().map(|ip| SocketAddr::new(ip, 0));
                Ok(Box::new(addresses.collect::<Vec<_>>().into_iter()) as Addrs)
            })
        }
    }

    async fn reach(server: &str, lookup: NameLookup) -> Reach {
        Reach::of(&Url::parse(server).unwrap(), lookup).await
    }

    async fn connects(reach: &Reach, name: &str) -> Vec<String> {
        reach
            .addresses(name.parse().unwrap())
            .await
            .map(|addresses| addresses.iter().map(|a| a.ip().to_string()).collect())
            .unwrap_or_default()
    }

    async fn refuses(reach: &Reach, name: &str) -> bool {
        match reach.addresses(name.parse().unwrap()).await {
            Ok(_) => false,
            Err(error) => refusal(error.as_ref()).is_some(),
        }
    }

    fn literal(reach: &Reach, url: &str) -> bool {
        reach.check_literal(&Url::parse(url).unwrap()).is_ok()
    }

    /// The table rmcp's own classes are tested by, so the two agree.
    #[test]
    fn an_address_is_classed_as_rmcp_classes_it() {
        let class = |ip: &str| AddressClass::of(ip.parse().unwrap());
        for public in [
            "93.184.216.34",
            "2606:4700::1111",
            "64:ff9b::5db8:d822",
            "100.63.255.255",
            "100.128.0.1",
        ] {
            assert_eq!(class(public), AddressClass::Public, "{public}");
        }
        for loopback in [
            "127.0.0.1",
            "127.8.9.10",
            "::1",
            "::ffff:127.0.0.1",
            "2002:7f00:1::",
        ] {
            assert_eq!(class(loopback), AddressClass::Loopback, "{loopback}");
        }
        for private in [
            "10.0.0.9",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.255.254",
            "fd00::1",
            "fc00::1",
            "::ffff:10.0.0.9",
            "::ffff:100.64.0.1",
            "64:ff9b::a00:9",
        ] {
            assert_eq!(class(private), AddressClass::Private, "{private}");
        }
        for blocked in [
            "169.254.169.254",
            "::ffff:169.254.169.254",
            "::169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::",
            "fe80::1",
            "0.0.0.0",
            "::",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "ff02::1",
            "198.18.0.1",
            "2001:0:4136:e378::1",
            "64:ff9b:1::a9fe:a9fe",
        ] {
            assert_eq!(class(blocked), AddressClass::Blocked, "{blocked}");
        }
    }

    #[tokio::test]
    async fn a_server_at_an_ip_address_is_placed_by_that_address() {
        let nothing = || table(&[]);
        let public = reach("https://93.184.216.34/mcp", nothing()).await;
        assert!(public.on_the_internet());
        assert!(!literal(&public, "http://127.0.0.1:6379/"));
        assert!(!literal(&public, "http://[::1]:6379/"));
        assert!(!literal(&public, "http://[::ffff:127.0.0.1]:6379/"));
        assert!(!literal(&public, "http://2130706433:6379/"));
        assert!(!literal(&public, "https://10.0.0.1/token"));
        assert!(!literal(&public, "https://[::ffff:10.0.0.1]/token"));
        assert!(!literal(&public, "https://100.64.0.7/token"));
        assert!(!literal(
            &public,
            "http://169.254.169.254/latest/meta-data/"
        ));
        assert!(literal(&public, "https://93.184.216.35/token"));
        assert!(literal(&public, "https://auth.example.com/token"));

        let loopback = reach("http://127.0.0.1:3000/mcp", nothing()).await;
        assert!(!loopback.on_the_internet());
        assert!(literal(&loopback, "http://127.0.0.2:8080/token"));
        assert!(literal(&loopback, "http://[::1]:8080/token"));
        assert!(literal(&loopback, "https://192.168.1.10/token"));
        assert!(literal(&loopback, "https://93.184.216.35/token"));
        assert!(!literal(&loopback, "http://169.254.169.254/"));

        let private = reach("https://[::ffff:10.0.0.4]/mcp", nothing()).await;
        assert!(!private.on_the_internet());
        assert!(literal(&private, "https://10.0.0.9/token"));
        assert!(literal(&private, "https://100.64.0.7/token"));
        assert!(literal(&private, "https://[fd00::9]/token"));
        assert!(literal(&private, "https://93.184.216.35/token"));
        assert!(!literal(&private, "http://127.0.0.1:6379/"));
        assert!(!literal(&private, "http://169.254.169.254/"));

        // A link-local server reaches itself and the internet, and nothing
        // else beside it.
        let link_local = reach("https://169.254.10.10/mcp", nothing()).await;
        assert!(link_local.on_the_internet());
        assert!(literal(&link_local, "https://169.254.10.10/token"));
        assert!(!literal(&link_local, "http://169.254.169.254/"));
        assert!(!literal(&link_local, "https://10.0.0.9/token"));
    }

    #[tokio::test]
    async fn a_server_by_name_is_placed_by_what_its_name_answers() {
        let lookup = || {
            table(&[
                ("mcp.example.com", &[&["93.184.216.34"]]),
                ("both.example.com", &[&["93.184.216.34", "10.0.0.1"]]),
                ("box.tailnet.test", &[&["100.101.102.103"]]),
                ("nas.local", &[&["192.168.1.20", "fe80::1"]]),
                ("dev.test", &[&["127.0.0.1", "::1"]]),
                ("mapped.test", &[&["::ffff:192.168.1.20"]]),
                ("imds.test", &[&["169.254.169.254"]]),
            ])
        };
        for (server, internet) in [
            ("https://mcp.example.com/mcp", true),
            // A name that answers both kinds is on the internet.
            ("https://both.example.com/mcp", true),
            ("https://box.tailnet.test/mcp", false),
            ("https://BOX.tailnet.test./mcp", false),
            ("https://nas.local/mcp", false),
            ("https://dev.test/mcp", false),
            ("https://mapped.test/mcp", false),
            // Link-local alone is nowhere a sign-in goes.
            ("https://imds.test/mcp", true),
            ("https://unanswered.test/mcp", true),
        ] {
            let placed = reach(server, lookup()).await;
            assert_eq!(placed.on_the_internet(), internet, "{server}");
        }

        let tailnet = reach("https://box.tailnet.test/mcp", lookup()).await;
        assert!(tailnet.private && !tailnet.loopback);
        let dev = reach("https://dev.test/mcp", lookup()).await;
        assert!(dev.private && dev.loopback);
        // Only the addresses that placed it are kept for it.
        let nas = reach("https://nas.local/mcp", lookup()).await;
        assert_eq!(connects(&nas, "nas.local").await, ["192.168.1.20"]);
    }

    #[tokio::test]
    async fn a_server_on_the_internet_connects_to_public_addresses_only() {
        let lookup = table(&[
            ("mcp.example.com", &[&["93.184.216.34"]]),
            ("auth.example.com", &[&["93.184.216.35"]]),
            ("loopback.attacker.test", &[&["127.0.0.1"]]),
            ("private.attacker.test", &[&["10.0.0.9"]]),
            ("tailnet.attacker.test", &[&["100.64.0.9"]]),
            ("mapped.attacker.test", &[&["::ffff:10.0.0.9"]]),
            ("imds.attacker.test", &[&["169.254.169.254"]]),
            (
                "mixed.attacker.test",
                &[&["127.0.0.1", "93.184.216.36", "10.0.0.1"]],
            ),
        ]);
        let public = reach("https://mcp.example.com/mcp", lookup).await;
        assert_eq!(
            connects(&public, "auth.example.com").await,
            ["93.184.216.35"]
        );
        for inward in [
            "loopback.attacker.test",
            "private.attacker.test",
            "tailnet.attacker.test",
            "mapped.attacker.test",
            "imds.attacker.test",
        ] {
            let error = public
                .addresses(inward.parse().unwrap())
                .await
                .expect_err(inward);
            let refused = refusal(error.as_ref()).expect("a refusal");
            assert!(refused.from_the_internet, "{inward}");
            assert_eq!(refused.to_string(), REFUSED_FROM_THE_INTERNET);
        }
        // Of an answer of both kinds, only the public address is used.
        assert_eq!(
            connects(&public, "mixed.attacker.test").await,
            ["93.184.216.36"]
        );
    }

    /// DNS rebinding, both ways round.
    #[tokio::test]
    async fn a_name_that_changes_its_answer_gains_nothing() {
        // Public for the check, inward for the connection: every lookup is
        // judged, the server's own name included.
        let lookup = table(&[
            (
                "mcp.attacker.test",
                &[&["93.184.216.34"], &["93.184.216.34"], &["127.0.0.1"]],
            ),
            ("auth.attacker.test", &[&["93.184.216.35"], &["10.0.0.9"]]),
        ]);
        let public = reach("https://mcp.attacker.test/mcp", lookup).await;
        assert_eq!(
            connects(&public, "mcp.attacker.test").await,
            ["93.184.216.34"]
        );
        assert!(refuses(&public, "mcp.attacker.test").await);
        assert_eq!(
            connects(&public, "auth.attacker.test").await,
            ["93.184.216.35"]
        );
        assert!(refuses(&public, "auth.attacker.test").await);

        // Private for the check, public afterwards: the name is never asked
        // for again, so what the internet serves under it is never read.
        let lookup = table(&[("mcp.attacker.test", &[&["10.0.0.5"], &["93.184.216.34"]])]);
        let held = reach("https://mcp.attacker.test/mcp", lookup).await;
        assert!(!held.on_the_internet());
        for _ in 0..3 {
            assert_eq!(connects(&held, "MCP.attacker.test.").await, ["10.0.0.5"]);
        }
    }

    #[tokio::test]
    async fn a_server_on_a_private_network_reaches_private_and_public_but_not_loopback() {
        let lookup = table(&[
            ("box.tailnet.test", &[&["100.101.102.103"]]),
            ("sso.corp.test", &[&["10.0.0.9"]]),
            ("login.example.com", &[&["93.184.216.35"]]),
            ("loopback.corp.test", &[&["127.0.0.1"]]),
            ("imds.corp.test", &[&["169.254.169.254"]]),
        ]);
        let private = reach("https://box.tailnet.test/mcp", lookup).await;
        assert_eq!(connects(&private, "sso.corp.test").await, ["10.0.0.9"]);
        // Its metadata may name an authorization server on the internet.
        assert_eq!(
            connects(&private, "login.example.com").await,
            ["93.184.216.35"]
        );
        for refused in ["loopback.corp.test", "imds.corp.test"] {
            let error = private
                .addresses(refused.parse().unwrap())
                .await
                .expect_err(refused);
            let refused = refusal(error.as_ref()).expect("a refusal");
            assert!(!refused.from_the_internet);
            assert_ne!(refused.to_string(), REFUSED_FROM_THE_INTERNET);
        }
        assert!(literal(&private, "https://10.0.0.9/token"));
        assert!(!literal(&private, "http://127.0.0.1:6379/"));
        assert!(!literal(&private, "http://169.254.169.254/"));
    }

    #[tokio::test]
    async fn where_a_request_would_connect_can_be_asked_before_it_is_made() {
        let lookup = || {
            table(&[
                ("mcp.example.com", &[&["93.184.216.34"]]),
                ("inward.attacker.test", &[&["192.168.1.1"]]),
                ("box.tailnet.test", &[&["100.101.102.103"]]),
            ])
        };
        let client = |server: &'static str| async move {
            SignInClient::for_server(&Url::parse(server).unwrap(), lookup())
                .await
                .unwrap()
        };
        let urls = |urls: &[&str]| urls.iter().map(ToString::to_string).collect::<Vec<_>>();

        let internet = client("https://mcp.example.com/mcp").await;
        let public = urls(&["https://mcp.example.com/token", "https://93.184.216.35/"]);
        assert!(internet.first_refused(&public).await.is_none());
        for inward in [
            "https://inward.attacker.test/register",
            "https://192.168.1.1/register",
            "http://[::ffff:127.0.0.1]:6379/",
        ] {
            let endpoints = urls(&["https://mcp.example.com/token", inward]);
            let refused = internet.first_refused(&endpoints).await.expect(inward);
            assert_eq!(refused.message(), REFUSED_FROM_THE_INTERNET, "{inward}");
        }

        let private = client("https://box.tailnet.test/mcp").await;
        let neighbours = urls(&[
            "https://box.tailnet.test/token",
            "https://inward.attacker.test/register",
            "https://192.168.1.1/register",
            "https://mcp.example.com/token",
        ]);
        assert!(private.first_refused(&neighbours).await.is_none());
        let link_local = urls(&["http://169.254.169.254/latest/meta-data/"]);
        let refused = private
            .first_refused(&link_local)
            .await
            .expect("link-local");
        assert_eq!(refused.message(), DISCOVERY_FAILED);
    }

    #[test]
    fn a_proxy_is_named_by_its_host() {
        let hosts = proxy_hosts(
            [
                "http://user:pass@Proxy.Corp:3128".to_string(),
                "bare-proxy.corp:8080".to_string(),
                "::not a url::".to_string(),
            ]
            .into_iter(),
        );
        assert_eq!(hosts, ["proxy.corp", "bare-proxy.corp"]);
    }

    #[tokio::test]
    async fn a_proxy_in_the_environment_is_connected_to_wherever_it_is() {
        let lookup = table(&[
            ("mcp.example.com", &[&["93.184.216.34"]]),
            ("proxy.corp", &[&["10.0.0.3"]]),
            ("private.attacker.test", &[&["10.0.0.9"]]),
        ]);
        let mut public = reach("https://mcp.example.com/mcp", lookup).await;
        public.proxies = vec!["proxy.corp".to_string()];
        assert_eq!(connects(&public, "proxy.corp").await, ["10.0.0.3"]);
        assert!(refuses(&public, "private.attacker.test").await);
    }
}
