use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

pub(super) const ALL: &str = "all";
/// The port a fresh install asks for, so a typed address can be a bare host.
/// A busy port falls back to an ephemeral one, and whatever was bound is saved.
pub(super) const DEFAULT_PORT: u16 = 8788;

pub(super) fn reachable(ip: IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    match ip {
        IpAddr::V4(ip) => !ip.is_link_local() && ip != Ipv4Addr::BROADCAST,
        // A phone cannot reuse this computer's interface scope for fe80:: URLs.
        IpAddr::V6(ip) => !ip.is_unicast_link_local() && ip.to_ipv4_mapped().is_none(),
    }
}

pub(super) fn addresses() -> Vec<String> {
    let mut ips: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|interface| interface.ip())
        .filter(|ip| reachable(*ip))
        .collect();
    ips.sort();
    ips.dedup();
    // UDP connect selects the local route without sending a packet. Prefer the
    // active LAN/VPN route over a Docker bridge when making the pairing QR.
    let primary = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
            socket.local_addr()
        })
        .ok()
        .map(|address| address.ip());
    if let Some(index) = ips.iter().position(|ip| Some(*ip) == primary) {
        let ip = ips.remove(index);
        ips.insert(0, ip);
    }
    ips.into_iter().map(|ip| ip.to_string()).collect()
}

pub(super) fn endpoint(host: &str, port: u16) -> String {
    let ip: IpAddr = host.parse().expect("validated interface address");
    format!("https://{}", SocketAddr::new(ip, port))
}
/// The bare host and port of an endpoint this module made.
pub(super) fn split(endpoint: &str) -> (String, u16) {
    let address: SocketAddr = endpoint
        .trim_start_matches("https://")
        .parse()
        .expect("an endpoint is a socket address");
    (address.ip().to_string(), address.port())
}

pub(super) async fn bind(
    host: &str,
    port: u16,
    addresses: &[String],
) -> std::io::Result<tokio::net::TcpListener> {
    if port == 0
        && let Ok(listener) = bind_once(host, DEFAULT_PORT, addresses).await
    {
        return Ok(listener);
    }
    bind_once(host, port, addresses).await
}
async fn bind_once(
    host: &str,
    port: u16,
    addresses: &[String],
) -> std::io::Result<tokio::net::TcpListener> {
    if host != ALL {
        return tokio::net::TcpListener::bind((host, port)).await;
    }
    let dual_stack = || {
        let socket = socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        socket.set_only_v6(false)?;
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port).into())?;
        socket.listen(128)?;
        tokio::net::TcpListener::from_std(socket.into())
    };
    match dual_stack() {
        Ok(listener) => Ok(listener),
        Err(error) if addresses.iter().any(|host| host.contains(':')) => Err(error),
        Err(_) => tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_addresses_are_routable_and_ipv6_urls_have_brackets() {
        for host in [
            "127.0.0.1",
            "127.2.3.4",
            "::1",
            "0.0.0.0",
            "::",
            "169.254.2.3",
            "fe80::1234",
            "224.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!reachable(host.parse().unwrap()), "{host}");
        }
        for host in ["192.168.1.5", "100.64.1.2", "fd00::42", "2001:db8::1"] {
            assert!(reachable(host.parse().unwrap()), "{host}");
        }
        assert_eq!(endpoint("fd00::42", 52261), "https://[fd00::42]:52261");
        assert_eq!(
            split("https://[fd00::42]:52261"),
            ("fd00::42".into(), 52261)
        );
        assert_eq!(
            split("https://192.168.1.5:8788"),
            ("192.168.1.5".into(), 8788)
        );
    }
}
