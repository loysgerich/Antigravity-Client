use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts, NameServerConfig, Protocol};
use hickory_resolver::TokioAsyncResolver;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Creates a custom DNS resolver that strictly uses xbox-dns.ru DoH and fallback IPs
pub fn create_custom_resolver() -> TokioAsyncResolver {
    let mut config = ResolverConfig::new();

    // 1. DoH (DNS over HTTPS)
    // The IP address of the DoH server. We'll use the primary IPv4 provided.
    // Alternatively, we can let it resolve the DoH domain using standard IPs first.
    let xbox_ip4_1 = IpAddr::V4(Ipv4Addr::new(111, 88, 96, 50));
    let xbox_ip4_2 = IpAddr::V4(Ipv4Addr::new(111, 88, 96, 51));
    let xbox_ip6_1 = IpAddr::V6(Ipv6Addr::new(0x2a00, 0xab00, 0x1233, 0x26, 0, 0, 0, 0x50));
    let xbox_ip6_2 = IpAddr::V6(Ipv6Addr::new(0x2a00, 0xab00, 0x1233, 0x26, 0, 0, 0, 0x51));

    // DoH Server 1
    let mut doh_server1 = NameServerConfig::new(SocketAddr::new(xbox_ip4_1, 443), Protocol::Https);
    doh_server1.tls_dns_name = Some("xbox-dns.ru".to_string());
    config.add_name_server(doh_server1);

    // UDP Fallbacks
    config.add_name_server(NameServerConfig::new(SocketAddr::new(xbox_ip4_1, 53), Protocol::Udp));
    config.add_name_server(NameServerConfig::new(SocketAddr::new(xbox_ip4_2, 53), Protocol::Udp));
    config.add_name_server(NameServerConfig::new(SocketAddr::new(xbox_ip6_1, 53), Protocol::Udp));
    config.add_name_server(NameServerConfig::new(SocketAddr::new(xbox_ip6_2, 53), Protocol::Udp));

    let mut opts = ResolverOpts::default();
    opts.use_hosts_file = false; // Strictly ignore OS hosts file
    opts.try_tcp_on_error = true;

    TokioAsyncResolver::tokio(config, opts)
}
