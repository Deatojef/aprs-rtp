use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    Error, Result,
    config::SourceConfig,
    rtp::{
        encoding::pt_info,
        packet,
        session::{AudioBlock, Session},
    },
};

/// Resolves `host` to an IPv4 multicast address.
async fn resolve_multicast(host: &str, port: u16) -> Result<Ipv4Addr> {
    let addr_str = if host.contains(':') {
        host.to_string()
    } else {
        format!("{}:{}", host, port)
    };

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(&addr_str)
        .await
        .map_err(|_| Error::ResolutionFailed(host.to_string()))?
        .collect();

    for addr in addrs {
        if let SocketAddr::V4(v4) = addr {
            let ip = *v4.ip();
            if ip.is_multicast() {
                return Ok(ip);
            }
            return Err(Error::NotMulticast(ip));
        }
    }
    Err(Error::ResolutionFailed(host.to_string()))
}

/// Bind a UDP socket and join an IPv4 multicast group.
fn join_multicast(group: Ipv4Addr, port: u16, iface: Ipv4Addr) -> Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(target_os = "linux")]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    socket.join_multicast_v4(&group, &iface)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

/// Spawn a tokio task that receives RTP packets from one multicast group,
/// demultiplexes by SSRC, and sends decoded `AudioBlock`s to the returned channel.
///
/// Returns a receiver that the DSP pipeline will consume.
pub async fn spawn(cfg: SourceConfig) -> Result<mpsc::Receiver<AudioBlock>> {
    let group = resolve_multicast(&cfg.host, cfg.port).await?;
    let iface = cfg
        .interface
        .as_deref()
        .and_then(|s| s.parse::<Ipv4Addr>().ok())
        .unwrap_or(Ipv4Addr::UNSPECIFIED);

    let std_sock = join_multicast(group, cfg.port, iface)?;
    let socket = tokio::net::UdpSocket::from_std(std_sock)?;

    info!(group = %group, port = cfg.port, "joined RTP multicast group");

    let (tx, rx) = mpsc::channel::<AudioBlock>(256);
    let jitter_buffer = cfg.jitter_buffer;
    let ssrc_filter: Vec<u32> = cfg.ssrc.clone();

    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        let mut sessions: HashMap<u32, Session> = HashMap::new();

        loop {
            let n = match socket.recv(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    warn!(error = %e, "UDP receive error");
                    continue;
                }
            };

            let (header, payload_offset) = match packet::parse(&buf[..n]) {
                Ok(v) => v,
                Err(e) => {
                    debug!(error = %e, "RTP parse error, dropping packet");
                    continue;
                }
            };

            // SSRC allowlist filter
            if !ssrc_filter.is_empty() && !ssrc_filter.contains(&header.ssrc) {
                continue;
            }

            let payload = &buf[payload_offset..n];

            let session = sessions.entry(header.ssrc).or_insert_with(|| {
                let info = match pt_info(header.payload_type) {
                    Some(i) => i,
                    None => {
                        // Return a temporary placeholder; we'll skip below.
                        return Session::new(
                            header.ssrc,
                            crate::rtp::encoding::PtInfo {
                                sample_rate: 0,
                                channels: 0,
                                encoding: crate::rtp::encoding::Encoding::S16Be,
                            },
                            jitter_buffer,
                        );
                    }
                };
                info!(
                    ssrc = header.ssrc,
                    pt = header.payload_type,
                    sample_rate = info.sample_rate,
                    channels = info.channels,
                    "new RTP session"
                );
                Session::new(header.ssrc, info, jitter_buffer)
            });

            // Skip sessions with unknown/unsupported payload type (sample_rate == 0).
            if session.pt_info.sample_rate == 0 {
                continue;
            }

            for block in session.ingest(&header, payload) {
                if tx.send(block).await.is_err() {
                    return; // receiver dropped, shut down
                }
            }
        }
    });

    Ok(rx)
}
