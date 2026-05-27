pub mod afsk;
pub mod aprs;
pub mod ax25;
pub mod config;
pub mod dsp;
pub mod error;
pub mod hdlc;
pub mod pipeline;
pub mod rtp;

pub use error::{Error, Result};

use std::time::SystemTime;
use tokio::sync::mpsc;

/// Audio level measurements at packet-decode time.
///
/// Mirrors direwolf's `alevel_t` and uses the same normalization:
///
/// - `rec`   = `(raw_peak − raw_valley) × 50`  — overall received level, 0–100 scale
///             where 100 = full-scale 16-bit audio.
/// - `mark`  = `mark_iq_peak × 100`             — 1200 Hz tone envelope, same scale.
/// - `space` = `space_iq_peak × 100`            — 2200 Hz tone envelope, same scale.
///
/// All three use a separate slower-tracking IIR (5× longer time constants than the
/// demodulation AGC) so values are stable across consecutive packets and can be
/// compared across different SSRCs on the same normalized audio scale.
///
/// Typical values for a well-adjusted APRS signal: rec 30–70, mark/space 10–40.
/// A pure full-scale tone yields mark/space ≈ 50 (IQ demodulation halves amplitude).
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioLevel {
    /// Overall received audio level (0–100, where 100 = full-scale S16 audio).
    pub rec: u8,
    /// Mark-tone (1200 Hz) IQ envelope level (0–100).
    pub mark: u8,
    /// Space-tone (2200 Hz) IQ envelope level (0–100).
    pub space: u8,
}

/// A decoded APRS packet ready for downstream consumers.
#[derive(Debug, Clone)]
pub struct AprsPacket {
    /// SSRC of the ka9q-radio audio channel this packet was decoded from.
    /// Maps 1:1 to a specific radiod demodulator channel (frequency).
    pub ssrc: u32,
    /// TNC2-format string: "SRC>DST,VIA,...:info"
    pub text: String,
    /// Validated AX.25 frame bytes excluding the FCS.
    /// All digipeater address H-bits are preserved for future heard-from analysis.
    pub raw_ax25: Vec<u8>,
    /// Wall-clock time the packet was decoded.
    pub received_at: SystemTime,
    /// Lowest-indexed slicer that successfully decoded this frame.
    pub first_slice: usize,
    /// Number of slicers (out of the configured total) that independently decoded
    /// this same frame within the same audio block.  Higher = stronger/cleaner signal.
    /// May undercount if slicers finish the frame across an audio-block boundary.
    pub slicer_hits: u8,
    /// Audio levels at decode time, normalized for cross-packet and cross-SSRC comparison.
    pub audio_level: AudioLevel,
    /// Tuned frequency in MHz, if provided in the source configuration.
    pub freq_mhz: Option<f64>,
}

/// Listens to one ka9q-radio RTP multicast group and decodes APRS packets.
///
/// Spawns one tokio async task that receives UDP audio and one blocking DSP thread
/// per active SSRC (per-channel demodulator + HDLC decoder + AX.25 parser).
///
/// Packets from multiple slicers decoding the same transmission are emitted
/// independently; deduplication is the responsibility of downstream consumers.
pub struct AprsListener {
    source: config::SourceConfig,
    decoder: config::DecoderConfig,
}

impl AprsListener {
    pub fn new(source: config::SourceConfig, decoder: config::DecoderConfig) -> Self {
        Self { source, decoder }
    }

    /// Spawn the listener and return a channel that yields decoded `AprsPacket`s.
    ///
    /// The channel remains open as long as the multicast socket is alive.
    pub async fn run(self) -> Result<mpsc::Receiver<AprsPacket>> {
        let (aprs_tx, aprs_rx) = mpsc::channel::<AprsPacket>(256);
        tokio::spawn(async move {
            if let Err(e) = pipeline::manager::run(self.source, self.decoder, aprs_tx).await {
                tracing::error!("pipeline manager error: {e}");
            }
        });
        Ok(aprs_rx)
    }
}
