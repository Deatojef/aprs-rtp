use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub decoder: DecoderConfig,
    #[serde(rename = "source")]
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    /// mDNS hostname (e.g. "packet.local") or raw multicast address (e.g. "239.0.0.1").
    pub host: String,
    /// RTP port. Default: 5004.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Network interface IP to bind to. Empty string or omitted = OS default.
    #[serde(default)]
    pub interface: Option<String>,
    /// Reorder window in RTP packets. Default: 2 (~80ms at 24kHz).
    #[serde(default = "default_jitter_buffer")]
    pub jitter_buffer: usize,
    /// SSRC allowlist. Empty = accept all SSRCs on the group.
    #[serde(default)]
    pub ssrc: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderConfig {
    /// Mark tone frequency in Hz. Default: 1200.
    #[serde(default = "default_mark_hz")]
    pub mark_hz: u32,
    /// Space tone frequency in Hz. Default: 2200.
    #[serde(default = "default_space_hz")]
    pub space_hz: u32,
    /// Baud rate. Default: 1200.
    #[serde(default = "default_baud")]
    pub baud: u32,
    /// Number of parallel amplitude-imbalance slicers (1–16). Default: 8.
    #[serde(default = "default_slicers")]
    pub slicers: usize,
    /// CRC error-recovery mode.
    #[serde(default)]
    pub fix_bits: FixBits,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            mark_hz: default_mark_hz(),
            space_hz: default_space_hz(),
            baud: default_baud(),
            slicers: default_slicers(),
            fix_bits: FixBits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FixBits {
    None,
    #[default]
    Single,
    Double,
}

fn default_port() -> u16 { 5004 }
fn default_jitter_buffer() -> usize { 2 }
fn default_mark_hz() -> u32 { 1200 }
fn default_space_hz() -> u32 { 2200 }
fn default_baud() -> u32 { 1200 }
fn default_slicers() -> usize { 8 }

impl Config {
    pub fn from_file(path: &str) -> crate::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| crate::Error::Config(format!("cannot read {path}: {e}")))?;
        toml::from_str(&text)
            .map_err(|e| crate::Error::Config(format!("parse error in {path}: {e}")))
    }
}
