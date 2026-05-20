use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub fn connect_urls(listen: &str, actual: SocketAddr) -> Vec<String> {
    let bind_host = listen
        .strip_prefix("ws://")
        .and_then(|rest| rest.rsplit_once(':').map(|(host, _)| host))
        .unwrap_or("127.0.0.1");

    if !is_wildcard_bind(bind_host) {
        return vec![format!(
            "ws://{}:{}",
            format_host(actual.ip()),
            actual.port()
        )];
    }

    connect_urls_for_hosts(actual.port(), discover_interface_hosts())
}

fn discover_interface_hosts() -> Vec<IpAddr> {
    let hosts = if_addrs::get_if_addrs()
        .map(|interfaces| {
            interfaces
                .into_iter()
                .map(|interface| interface.addr.ip())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if hosts.is_empty() {
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
    } else {
        hosts
    }
}

fn connect_urls_for_hosts(port: u16, hosts: Vec<IpAddr>) -> Vec<String> {
    let mut hosts = hosts
        .into_iter()
        .filter(|ip| is_tailscale(*ip) || is_private_lan(*ip) || ip.is_loopback())
        .collect::<Vec<_>>();
    hosts.sort_by_key(|ip| (host_rank(*ip), ip.to_string()));
    hosts.dedup();

    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for host in hosts {
        let url = format!("ws://{}:{port}", format_host(host));
        if seen.insert(url.clone()) {
            urls.push(url);
        }
    }

    if urls.is_empty() {
        urls.push(format!("ws://127.0.0.1:{port}"));
    }
    urls
}

fn is_wildcard_bind(host: &str) -> bool {
    matches!(host, "0.0.0.0" | "::" | "[::]")
}

fn host_rank(ip: IpAddr) -> u8 {
    if is_tailscale(ip) {
        0
    } else if is_private_lan(ip) {
        1
    } else if ip.is_loopback() {
        2
    } else {
        3
    }
}

fn is_tailscale(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            octets[0] == 100 && (64..=127).contains(&octets[1])
        }
        IpAddr::V6(_) => false,
    }
}

fn is_private_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private(),
        IpAddr::V6(ip) => ip.is_unique_local(),
    }
}

fn format_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_connect_urls_sort_tailscale_lan_then_loopback() {
        let urls = connect_urls_for_hosts(
            4321,
            vec![
                "127.0.0.1".parse().unwrap(),
                "192.168.1.12".parse().unwrap(),
                "100.100.10.20".parse().unwrap(),
                "8.8.8.8".parse().unwrap(),
                "10.0.0.2".parse().unwrap(),
            ],
        );

        assert_eq!(
            urls,
            vec![
                "ws://100.100.10.20:4321",
                "ws://10.0.0.2:4321",
                "ws://192.168.1.12:4321",
                "ws://127.0.0.1:4321",
            ]
        );
    }

    #[test]
    fn wildcard_connect_urls_fall_back_to_loopback() {
        let urls = connect_urls_for_hosts(4321, vec!["8.8.8.8".parse().unwrap()]);
        assert_eq!(urls, vec!["ws://127.0.0.1:4321"]);
    }

    #[test]
    fn explicit_bind_uses_actual_bound_host() {
        let actual = "127.0.0.1:4321".parse().unwrap();
        assert_eq!(
            connect_urls("ws://127.0.0.1:0", actual),
            vec!["ws://127.0.0.1:4321"]
        );
    }
}
