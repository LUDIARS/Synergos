//! 最小 STUN クライアント (RFC 5389) — Binding Request で自分の public address を発見する。
//!
//! スコープ:
//! - Binding Request のみ (Allocate / Refresh 等の TURN 拡張は `turn.rs` 側)
//! - XOR-MAPPED-ADDRESS 属性だけを解釈 (IPv4 / IPv6 両対応)
//! - 認証なし (classic STUN の discovery 用途)
//!
//! STUN メッセージは固定 20 byte ヘッダ + 可変 TLV 属性:
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |0 0|     STUN Message Type     |         Message Length        |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                         Magic Cookie                          |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                                                               |
//! |                     Transaction ID (96 bits)                  |
//! |                                                               |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::error::{Result, SynergosNetError};

const MAGIC_COOKIE: u32 = 0x2112_A442;
const TYPE_BINDING_REQUEST: u16 = 0x0001;
const TYPE_BINDING_SUCCESS: u16 = 0x0101;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const FAMILY_IPV4: u8 = 0x01;
const FAMILY_IPV6: u8 = 0x02;

/// STUN Binding Request を送り、応答の XOR-MAPPED-ADDRESS を返す。
/// `server_addr` は STUN サーバ (通常 UDP port 3478) の UDP アドレス。
pub async fn discover_public_address(
    server_addr: SocketAddr,
    timeout: Duration,
) -> Result<SocketAddr> {
    // local UDP socket を bind (IPv4/IPv6 自動)
    let bind_addr = if server_addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let sock = UdpSocket::bind(bind_addr)
        .await
        .map_err(|e| SynergosNetError::Turn(format!("stun bind: {e}")))?;
    sock.connect(server_addr)
        .await
        .map_err(|e| SynergosNetError::Turn(format!("stun connect: {e}")))?;

    // Binding Request を組み立て
    let tx_id = random_tx_id();
    let req = encode_binding_request(&tx_id);
    sock.send(&req)
        .await
        .map_err(|e| SynergosNetError::Turn(format!("stun send: {e}")))?;

    let mut buf = [0u8; 1500];
    let n = tokio::time::timeout(timeout, sock.recv(&mut buf))
        .await
        .map_err(|_| SynergosNetError::Turn("stun timeout".into()))?
        .map_err(|e| SynergosNetError::Turn(format!("stun recv: {e}")))?;

    parse_binding_success(&buf[..n], &tx_id)
}

fn random_tx_id() -> [u8; 12] {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut tx = [0u8; 12];
    rng.fill_bytes(&mut tx);
    tx
}

fn encode_binding_request(tx_id: &[u8; 12]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    out.extend_from_slice(&TYPE_BINDING_REQUEST.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // length = 0 (no attrs)
    out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    out.extend_from_slice(tx_id);
    out
}

fn parse_binding_success(bytes: &[u8], expected_tx: &[u8; 12]) -> Result<SocketAddr> {
    if bytes.len() < 20 {
        return Err(SynergosNetError::Turn("stun response too short".into()));
    }
    let msg_type = u16::from_be_bytes([bytes[0], bytes[1]]);
    if msg_type != TYPE_BINDING_SUCCESS {
        return Err(SynergosNetError::Turn(format!(
            "unexpected STUN message type: {msg_type:#06x}"
        )));
    }
    let msg_len = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    let cookie = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if cookie != MAGIC_COOKIE {
        return Err(SynergosNetError::Turn("invalid magic cookie".into()));
    }
    if &bytes[8..20] != expected_tx {
        return Err(SynergosNetError::Turn("transaction id mismatch".into()));
    }
    if bytes.len() < 20 + msg_len {
        return Err(SynergosNetError::Turn("stun body truncated".into()));
    }

    // 属性を走査
    let mut cursor = 20;
    let end = 20 + msg_len;
    while cursor + 4 <= end {
        let attr_type = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]);
        let attr_len = u16::from_be_bytes([bytes[cursor + 2], bytes[cursor + 3]]) as usize;
        let val_start = cursor + 4;
        let val_end = val_start + attr_len;
        if val_end > end {
            return Err(SynergosNetError::Turn("attribute past message end".into()));
        }

        if attr_type == ATTR_XOR_MAPPED_ADDRESS {
            return parse_xor_mapped(&bytes[val_start..val_end], expected_tx);
        }

        // 4 byte 境界で padding
        let padded = (attr_len + 3) & !3;
        cursor = val_start + padded;
    }

    Err(SynergosNetError::Turn(
        "no XOR-MAPPED-ADDRESS attribute".into(),
    ))
}

fn parse_xor_mapped(attr: &[u8], tx_id: &[u8; 12]) -> Result<SocketAddr> {
    if attr.len() < 4 {
        return Err(SynergosNetError::Turn("xor-mapped too short".into()));
    }
    let family = attr[1];
    let xor_port = u16::from_be_bytes([attr[2], attr[3]]);
    let port = xor_port ^ ((MAGIC_COOKIE >> 16) as u16);

    match family {
        FAMILY_IPV4 => {
            if attr.len() < 8 {
                return Err(SynergosNetError::Turn("xor-mapped ipv4 too short".into()));
            }
            let xor_ip = u32::from_be_bytes([attr[4], attr[5], attr[6], attr[7]]);
            let ip = Ipv4Addr::from(xor_ip ^ MAGIC_COOKIE);
            Ok(SocketAddr::new(IpAddr::V4(ip), port))
        }
        FAMILY_IPV6 => {
            if attr.len() < 20 {
                return Err(SynergosNetError::Turn("xor-mapped ipv6 too short".into()));
            }
            let mut ip_bytes = [0u8; 16];
            ip_bytes.copy_from_slice(&attr[4..20]);
            // XOR with magic cookie || tx_id
            let mut xor_key = [0u8; 16];
            xor_key[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
            xor_key[4..].copy_from_slice(tx_id);
            for (a, b) in ip_bytes.iter_mut().zip(xor_key.iter()) {
                *a ^= *b;
            }
            Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip_bytes)), port))
        }
        other => Err(SynergosNetError::Turn(format!(
            "unknown address family {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_request_is_20_bytes() {
        let tx = [0xAAu8; 12];
        let req = encode_binding_request(&tx);
        assert_eq!(req.len(), 20);
        assert_eq!(&req[0..2], &TYPE_BINDING_REQUEST.to_be_bytes());
        assert_eq!(&req[2..4], &[0, 0]); // length
        assert_eq!(&req[4..8], &MAGIC_COOKIE.to_be_bytes());
        assert_eq!(&req[8..20], &tx);
    }

    #[test]
    fn parses_ipv4_xor_mapped_address() {
        // 実際の STUN サーバ応答を模倣: 192.0.2.1:8080
        let tx = [0x12u8; 12];
        let mut resp = Vec::new();
        resp.extend_from_slice(&TYPE_BINDING_SUCCESS.to_be_bytes());
        resp.extend_from_slice(&12u16.to_be_bytes()); // body len
        resp.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        resp.extend_from_slice(&tx);
        // Attr: XOR-MAPPED-ADDRESS
        resp.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        resp.extend_from_slice(&8u16.to_be_bytes()); // 8 bytes value
        resp.push(0);
        resp.push(FAMILY_IPV4);
        let port = 8080u16;
        let xport = port ^ ((MAGIC_COOKIE >> 16) as u16);
        resp.extend_from_slice(&xport.to_be_bytes());
        let ip = Ipv4Addr::new(192, 0, 2, 1);
        let xip = u32::from(ip) ^ MAGIC_COOKIE;
        resp.extend_from_slice(&xip.to_be_bytes());

        let addr = parse_binding_success(&resp, &tx).unwrap();
        assert_eq!(addr, SocketAddr::new(IpAddr::V4(ip), port));
    }

    #[test]
    fn rejects_wrong_transaction_id() {
        let tx = [0x11u8; 12];
        let wrong_tx = [0x22u8; 12];
        let mut resp = Vec::new();
        resp.extend_from_slice(&TYPE_BINDING_SUCCESS.to_be_bytes());
        resp.extend_from_slice(&0u16.to_be_bytes());
        resp.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        resp.extend_from_slice(&tx);

        let err = parse_binding_success(&resp, &wrong_tx).unwrap_err();
        match err {
            SynergosNetError::Turn(msg) => assert!(msg.contains("transaction id")),
            other => panic!("expected Turn, got {other:?}"),
        }
    }

    #[test]
    fn rejects_wrong_magic_cookie() {
        let tx = [0u8; 12];
        let mut resp = Vec::new();
        resp.extend_from_slice(&TYPE_BINDING_SUCCESS.to_be_bytes());
        resp.extend_from_slice(&0u16.to_be_bytes());
        resp.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        resp.extend_from_slice(&tx);
        assert!(parse_binding_success(&resp, &tx).is_err());
    }

    // ループバック UDP 上で STUN サーバを模倣して discover_public_address を回す
    #[tokio::test]
    async fn discover_against_mock_stun_server() {
        // バインドしてポートを得る
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let mut buf = [0u8; 1500];
            let (n, peer) = server.recv_from(&mut buf).await.unwrap();
            // Binding Request を受けた前提で Binding Success を返す
            let tx_id: [u8; 12] = buf[8..20].try_into().unwrap();
            let mut resp = Vec::new();
            resp.extend_from_slice(&TYPE_BINDING_SUCCESS.to_be_bytes());
            resp.extend_from_slice(&12u16.to_be_bytes());
            resp.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
            resp.extend_from_slice(&tx_id);
            resp.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
            resp.extend_from_slice(&8u16.to_be_bytes());
            resp.push(0);
            resp.push(FAMILY_IPV4);
            let port = match peer.ip() {
                IpAddr::V4(_) => peer.port(),
                _ => 0,
            };
            let xport = port ^ ((MAGIC_COOKIE >> 16) as u16);
            resp.extend_from_slice(&xport.to_be_bytes());
            let ip = match peer.ip() {
                IpAddr::V4(v4) => u32::from(v4),
                _ => 0,
            };
            let xip = ip ^ MAGIC_COOKIE;
            resp.extend_from_slice(&xip.to_be_bytes());
            server.send_to(&resp, peer).await.unwrap();
            let _ = n;
        });

        let result = discover_public_address(server_addr, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(result.is_ipv4());
        server_task.await.unwrap();
    }
}
