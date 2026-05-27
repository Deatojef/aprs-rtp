use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::mpsc;

use crate::{
    afsk::AfskDemodulator,
    aprs::text::to_tnc2,
    ax25::frame::Ax25Frame,
    config::{DecoderConfig, FixBits},
    hdlc::{fec::try_validate, framer::HdlcDecoder},
    rtp::session::AudioBlock,
    AprsPacket,
};

// Suppress re-emission of a frame with identical raw AX.25 bytes from the same
// SSRC for this long.  Covers cross-block slicer phase differences (which are
// milliseconds) while well within the minimum APRS repeat interval (~30 s).
const DEDUP_WINDOW: Duration = Duration::from_secs(3);

/// All DSP state for one SSRC audio stream.
///
/// Lives in a `spawn_blocking` thread; fed via a `std::sync::mpsc` channel.
/// One `StreamDecoder` per active SSRC.
pub struct StreamDecoder {
    ssrc: u32,
    freq_mhz: Option<f64>,
    demod: AfskDemodulator,
    hdlc: Vec<HdlcDecoder>,
    fix_bits: FixBits,
    out: mpsc::Sender<AprsPacket>,
    /// Content-hash dedup cache: raw_ax25 bytes → time last emitted.
    dedup_cache: HashMap<Vec<u8>, Instant>,
}

impl StreamDecoder {
    pub fn new(
        ssrc: u32,
        cfg: &DecoderConfig,
        sample_rate: u32,
        freq_mhz: Option<f64>,
        out: mpsc::Sender<AprsPacket>,
    ) -> Self {
        let num_slicers = cfg.slicers;
        let hdlc = (0..num_slicers).map(HdlcDecoder::new).collect();
        Self {
            ssrc,
            freq_mhz,
            demod: AfskDemodulator::new(cfg, sample_rate),
            hdlc,
            fix_bits: cfg.fix_bits,
            out,
            dedup_cache: HashMap::new(),
        }
    }

    /// Process one `AudioBlock` synchronously (call from a blocking context).
    ///
    /// Returns `false` when the output channel is closed (caller should stop).
    ///
    /// Within-block dedup: multiple slicers that decode the same physical frame
    /// in the same block are merged into one `AprsPacket` (`slicer_hits` counts
    /// them all).  Cross-block dedup: frames whose raw bytes appeared within the
    /// last `DEDUP_WINDOW` are suppressed entirely.
    pub fn process_block(&mut self, block: &AudioBlock) -> bool {
        let now = Instant::now();

        // Evict stale dedup entries once per block (cache stays tiny in practice).
        self.dedup_cache.retain(|_, seen_at| now.duration_since(*seen_at) < DEDUP_WINDOW);

        // Collect all valid frames decoded this block.
        // Key: raw_ax25 bytes.  Value: (tnc2_text, first_slice, slicer_hit_count).
        let mut decoded: HashMap<Vec<u8>, (String, usize, u8)> = HashMap::new();

        for &sample in &block.samples {
            let bits = self.demod.process_sample(sample);
            for demod_bit in &bits {
                let slicer_idx = demod_bit.slice;
                if slicer_idx >= self.hdlc.len() {
                    continue;
                }
                if let Some(raw) = self.hdlc[slicer_idx].push(demod_bit) {
                    if let Some(valid) = try_validate(&raw, self.fix_bits) {
                        if let Some(ax25) = Ax25Frame::parse(&valid.data) {
                            let text = to_tnc2(&ax25);
                            let e = decoded
                                .entry(valid.data)
                                .or_insert((text, slicer_idx, 0u8));
                            if slicer_idx < e.1 {
                                e.1 = slicer_idx; // track lowest slicer index
                            }
                            e.2 = e.2.saturating_add(1);
                        }
                    }
                }
            }
        }

        // Snapshot audio levels at end of block.
        let audio_level = self.demod.audio_level();

        for (raw_ax25, (text, first_slice, slicer_hits)) in decoded {
            // Cross-block dedup: suppress if we emitted the same frame recently.
            if self.dedup_cache.contains_key(&raw_ax25) {
                continue;
            }
            self.dedup_cache.insert(raw_ax25.clone(), now);

            let pkt = AprsPacket {
                ssrc: self.ssrc,
                text,
                raw_ax25,
                received_at: SystemTime::now(),
                first_slice,
                slicer_hits,
                audio_level,
                freq_mhz: self.freq_mhz,
            };
            if self.out.blocking_send(pkt).is_err() {
                return false;
            }
        }

        true
    }
}

/// Spawn a blocking DSP task for one SSRC.
///
/// Returns a `std::sync::mpsc::SyncSender` that the async caller uses to push
/// `AudioBlock`s into the blocking thread. The blocking thread processes each
/// block and forwards decoded `AprsPacket`s to `out`.
///
/// The blocking thread exits when the sender side is dropped or when `out` closes.
pub fn spawn(
    ssrc: u32,
    cfg: DecoderConfig,
    sample_rate: u32,
    freq_mhz: Option<f64>,
    out: mpsc::Sender<AprsPacket>,
) -> std::sync::mpsc::SyncSender<AudioBlock> {
    // Bounded sync channel: limit backlog to 4 blocks (~160ms at 24kHz/960-sample blocks).
    let (tx, rx) = std::sync::mpsc::sync_channel::<AudioBlock>(4);

    tokio::task::spawn_blocking(move || {
        let mut dec = StreamDecoder::new(ssrc, &cfg, sample_rate, freq_mhz, out);
        while let Ok(block) = rx.recv() {
            if !dec.process_block(&block) {
                break;
            }
        }
        tracing::debug!(ssrc, "stream decoder exiting");
    });

    tx
}
