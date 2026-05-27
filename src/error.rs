use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("RTP version {0}, expected 2")]
    RtpVersion(u8),
    #[error("unsupported RTP payload type {0}")]
    UnsupportedEncoding(u8),
    #[error("RTP packet too short: {0} bytes")]
    PacketTooShort(usize),
    #[error("hostname resolution failed for '{0}'")]
    ResolutionFailed(String),
    #[error("multicast address required, got unicast {0}")]
    NotMulticast(std::net::Ipv4Addr),
    #[error("channel closed")]
    ChannelClosed,
    #[error("configuration error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;
