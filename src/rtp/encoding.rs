/// PCM encoding format carried in ka9q-radio RTP streams.
///
/// All payload types in ka9q-radio's PCM table use S16Be (big-endian signed 16-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    S16Be,
}

/// Describes an RTP payload type's audio format.
#[derive(Debug, Clone, Copy)]
pub struct PtInfo {
    pub sample_rate: u32,
    pub channels: u8,
    pub encoding: Encoding,
}

/// Look up a ka9q-radio RTP payload type.
///
/// Returns `None` for unknown or unsupported types (e.g. Opus, MULAW, AX25).
/// Only PCM types relevant to AFSK decoding are returned.
pub fn pt_info(pt: u8) -> Option<PtInfo> {
    // PT table from ka9q-radio src/rtp.c, PCM-only entries.
    // PT 116 (24kHz mono S16BE) is the primary target for the fm preset.
    match pt {
        10 => Some(PtInfo {
            sample_rate: 44100,
            channels: 2,
            encoding: Encoding::S16Be,
        }),
        11 => Some(PtInfo {
            sample_rate: 44100,
            channels: 1,
            encoding: Encoding::S16Be,
        }),
        112 => Some(PtInfo {
            sample_rate: 48000,
            channels: 1,
            encoding: Encoding::S16Be,
        }),
        113 => Some(PtInfo {
            sample_rate: 48000,
            channels: 2,
            encoding: Encoding::S16Be,
        }),
        116 => Some(PtInfo {
            sample_rate: 24000,
            channels: 1,
            encoding: Encoding::S16Be,
        }),
        117 => Some(PtInfo {
            sample_rate: 24000,
            channels: 2,
            encoding: Encoding::S16Be,
        }),
        119 => Some(PtInfo {
            sample_rate: 16000,
            channels: 1,
            encoding: Encoding::S16Be,
        }),
        120 => Some(PtInfo {
            sample_rate: 16000,
            channels: 2,
            encoding: Encoding::S16Be,
        }),
        122 => Some(PtInfo {
            sample_rate: 12000,
            channels: 1,
            encoding: Encoding::S16Be,
        }),
        123 => Some(PtInfo {
            sample_rate: 12000,
            channels: 2,
            encoding: Encoding::S16Be,
        }),
        125 => Some(PtInfo {
            sample_rate: 8000,
            channels: 1,
            encoding: Encoding::S16Be,
        }),
        126 => Some(PtInfo {
            sample_rate: 8000,
            channels: 2,
            encoding: Encoding::S16Be,
        }),
        _ => None,
    }
}

/// Decode a PCM payload into normalized f32 samples in the range [-1.0, 1.0].
///
/// For stereo encodings only the left channel (even samples) is returned,
/// since APRS is always mono.
pub fn decode_samples(payload: &[u8], info: &PtInfo) -> Vec<f32> {
    let Encoding::S16Be = info.encoding;
    payload
        .chunks_exact(2 * info.channels as usize)
        .map(|chunk| {
            let s = i16::from_be_bytes([chunk[0], chunk[1]]);
            s as f32 / 32768.0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pt116_is_24khz_mono_s16be() {
        let info = pt_info(116).unwrap();
        assert_eq!(info.sample_rate, 24000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.encoding, Encoding::S16Be);
    }

    #[test]
    fn s16be_full_scale() {
        let info = PtInfo {
            sample_rate: 24000,
            channels: 1,
            encoding: Encoding::S16Be,
        };
        // i16::MAX = 32767 → ~1.0; i16::MIN = -32768 → -1.0
        let payload = [0x7F, 0xFF, 0x80, 0x00]; // 32767, -32768
        let samples = decode_samples(&payload, &info);
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - (32767.0 / 32768.0)).abs() < 1e-5);
        assert!((samples[1] - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn stereo_s16be_extracts_left_channel() {
        // Stereo: left=0x7FFF, right=0x0000 per frame
        let info = PtInfo {
            sample_rate: 24000,
            channels: 2,
            encoding: Encoding::S16Be,
        };
        let payload = [0x7F, 0xFF, 0x00, 0x00];
        let samples = decode_samples(&payload, &info);
        assert_eq!(samples.len(), 1);
        assert!((samples[0] - (32767.0 / 32768.0)).abs() < 1e-5);
    }
}
