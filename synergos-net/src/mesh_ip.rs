//! Cloudflare Mesh の仮想 IP (CGNAT レンジ `100.96.0.0/12`) の自己検出。
//!
//! Mesh 参加ノードは WARP が作る仮想インターフェースにこのレンジのアドレスを
//! 持つ。管制サーバーへの heartbeat 報告や `quic_advertised_addr` の確認に使う。

use std::net::Ipv4Addr;

/// Cloudflare Mesh のデバイス IP 既定レンジ `100.96.0.0/12` に含まれるか。
pub fn is_mesh_ipv4(addr: Ipv4Addr) -> bool {
    // /12 = 上位 12bit 一致 (100.96.0.0 〜 100.111.255.255)
    let octets = addr.octets();
    octets[0] == 100 && (96..=111).contains(&octets[1])
}

/// ローカル NIC を列挙して Mesh レンジの IPv4 を返す。参加していなければ None。
pub fn detect_mesh_ipv4() -> Option<Ipv4Addr> {
    let interfaces = if_addrs::get_if_addrs().ok()?;
    interfaces.into_iter().find_map(|iface| match iface.ip() {
        std::net::IpAddr::V4(v4) if is_mesh_ipv4(v4) => Some(v4),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_range_boundaries() {
        assert!(is_mesh_ipv4("100.96.0.0".parse().unwrap()));
        assert!(is_mesh_ipv4("100.111.255.255".parse().unwrap()));
        assert!(is_mesh_ipv4("100.100.1.2".parse().unwrap()));
        assert!(!is_mesh_ipv4("100.95.255.255".parse().unwrap()));
        assert!(!is_mesh_ipv4("100.112.0.0".parse().unwrap()));
        assert!(!is_mesh_ipv4("10.0.0.1".parse().unwrap()));
        assert!(!is_mesh_ipv4("192.168.1.1".parse().unwrap()));
    }
}
