use std::collections::BTreeMap;
use crate::rtp::{encoding::{self, PtInfo}, packet::RtpHeader};

/// Maximum silence samples to inject for a gap before treating it as a stream restart.
/// At 24kHz this is 80ms — enough to cover jitter but short enough to avoid
/// feeding the DPLL endless silence after a signal loss.
const MAX_GAP_SAMPLES: usize = 1920;

/// State for one SSRC within a multicast group.
pub struct Session {
    pub ssrc: u32,
    pub pt_info: PtInfo,
    next_seq: u16,
    initialized: bool,
    /// Reorder buffer: holds up to `jitter_buffer` out-of-order packets.
    reorder: BTreeMap<u16, Vec<u8>>,
    jitter_buffer: usize,
    /// Samples per RTP packet, learned from the first decoded packet.
    samples_per_packet: usize,
}

/// Audio block ready for the DSP pipeline.
pub struct AudioBlock {
    pub ssrc: u32,
    pub sample_rate: u32,
    /// Normalized f32 samples in [-1.0, 1.0].
    pub samples: Vec<f32>,
}

impl Session {
    pub fn new(ssrc: u32, pt_info: PtInfo, jitter_buffer: usize) -> Self {
        Self {
            ssrc,
            pt_info,
            next_seq: 0,
            initialized: false,
            reorder: BTreeMap::new(),
            jitter_buffer,
            samples_per_packet: 0,
        }
    }

    /// Ingest a raw RTP payload for this session.
    ///
    /// Returns zero or more `AudioBlock`s: normally one per call, but may return
    /// multiple when flushing the reorder buffer, and may include silence-padded
    /// blocks for gaps.
    pub fn ingest(&mut self, header: &RtpHeader, payload: &[u8]) -> Vec<AudioBlock> {
        // Insert into the reorder buffer.
        self.reorder.insert(header.seq, payload.to_vec());

        // Bootstrap: don't start processing until the reorder window is filled.
        // This lets out-of-order packets settle before we commit to a sequence.
        if !self.initialized {
            if self.reorder.len() > self.jitter_buffer {
                self.next_seq = *self.reorder.keys().next().unwrap();
                self.initialized = true;
            } else {
                return Vec::new();
            }
        }

        let mut out = Vec::new();

        // Flush packets that are ready: oldest held packet when buffer exceeds the window.
        loop {
            let oldest_seq = match self.reorder.keys().next().copied() {
                Some(s) => s,
                None => break,
            };

            if self.reorder.len() <= self.jitter_buffer {
                break;
            }

            let gap = seq_distance(oldest_seq, self.next_seq);

            if gap > 0 {
                // Fill gap with silence up to MAX_GAP_SAMPLES.
                let gap_samples = gap as usize * self.samples_per_packet;
                if gap_samples > 0 && gap_samples <= MAX_GAP_SAMPLES {
                    out.push(AudioBlock {
                        ssrc: self.ssrc,
                        sample_rate: self.pt_info.sample_rate,
                        samples: vec![0.0f32; gap_samples],
                    });
                } else if gap_samples > MAX_GAP_SAMPLES {
                    // Large gap: treat as stream restart — skip ahead silently.
                    tracing::debug!(ssrc = self.ssrc, gap, "large gap, resetting sequence");
                }
                self.next_seq = oldest_seq;
            }

            let payload = self.reorder.remove(&oldest_seq).unwrap();
            let samples = encoding::decode_samples(&payload, &self.pt_info);

            if self.samples_per_packet == 0 && !samples.is_empty() {
                self.samples_per_packet = samples.len();
            }

            self.next_seq = self.next_seq.wrapping_add(1);

            tracing::trace!(ssrc = self.ssrc, seq = oldest_seq, n = samples.len(), "audio block");

            out.push(AudioBlock {
                ssrc: self.ssrc,
                sample_rate: self.pt_info.sample_rate,
                samples,
            });
        }

        out
    }
}

/// Signed distance from `expected` to `actual` in u16 sequence space.
/// Positive means `actual` is ahead; negative means it's behind (duplicate/reorder).
fn seq_distance(actual: u16, expected: u16) -> i32 {
    actual.wrapping_sub(expected) as i16 as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtp::encoding::{Encoding, PtInfo};

    fn make_pt() -> PtInfo {
        PtInfo { sample_rate: 24000, channels: 1, encoding: Encoding::S16Be }
    }

    fn make_header(seq: u16, ssrc: u32) -> RtpHeader {
        RtpHeader { payload_type: 116, seq, ssrc }
    }

    fn make_payload(n_samples: usize) -> Vec<u8> {
        // All-zero S16BE samples
        vec![0u8; n_samples * 2]
    }

    #[test]
    fn in_order_packets_produce_blocks() {
        let mut session = Session::new(1, make_pt(), 2);
        let blocks = session.ingest(&make_header(0, 1), &make_payload(480));
        // With jitter_buffer=2 and only 1 packet, nothing flushed yet.
        assert!(blocks.is_empty());

        let blocks = session.ingest(&make_header(1, 1), &make_payload(480));
        assert!(blocks.is_empty());

        // Third packet pushes reorder buffer over jitter_buffer (2), flushing seq=0.
        let blocks = session.ingest(&make_header(2, 1), &make_payload(480));
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].samples.len(), 480);
    }

    #[test]
    fn gap_produces_silence() {
        let mut session = Session::new(1, make_pt(), 0);
        // jitter_buffer=0: flush immediately
        session.ingest(&make_header(0, 1), &make_payload(480));

        // Seq 1 missing; seq 2 arrives.
        let blocks = session.ingest(&make_header(2, 1), &make_payload(480));
        // Should produce silence for seq 1, then the seq 2 audio.
        // samples_per_packet was learned as 480 from seq 0.
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].samples.iter().all(|&s| s == 0.0)); // silence
        assert_eq!(blocks[0].samples.len(), 480);
        assert_eq!(blocks[1].samples.len(), 480);
    }

    #[test]
    fn seq_distance_wraps() {
        assert_eq!(seq_distance(1, 0), 1);
        assert_eq!(seq_distance(0, 1), -1);
        // Wrap: seq 0 after seq 65535
        assert_eq!(seq_distance(0, 65535), 1);
    }
}
