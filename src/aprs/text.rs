use crate::ax25::frame::Ax25Frame;

/// Format an `Ax25Frame` as a TNC2 text line.
///
/// Format: `SOURCE>DESTINATION[,VIA...]:info`
///
/// Digipeater entries that have been repeated (H-bit set) are suffixed with `*`.
/// This matches the standard TNC2 monitor format used by APRS software.
pub fn to_tnc2(frame: &Ax25Frame) -> String {
    let mut s = String::with_capacity(128);
    s.push_str(&frame.source);
    s.push('>');
    s.push_str(&frame.destination);

    for (call, &heard) in frame.via.iter().zip(frame.via_heard.iter()) {
        s.push(',');
        s.push_str(call);
        if heard {
            s.push('*');
        }
    }

    s.push(':');

    // Append info field; filter out non-printable control bytes except tab.
    for &byte in &frame.info {
        if byte == b'\t' || (byte >= 0x20 && byte < 0x7F) || byte >= 0x80 {
            s.push(byte as char);
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax25::frame::Ax25Frame;

    fn make_frame(
        src: &str,
        dst: &str,
        via: Vec<(String, bool)>,
        info: &[u8],
    ) -> Ax25Frame {
        let (via_calls, via_heard): (Vec<String>, Vec<bool>) = via.into_iter().unzip();
        Ax25Frame {
            source: src.to_string(),
            destination: dst.to_string(),
            via: via_calls,
            via_heard,
            info: info.to_vec(),
        }
    }

    #[test]
    fn direct_packet_no_via() {
        let f = make_frame("KA9Q-1", "APDR15", vec![], b"/test info");
        assert_eq!(to_tnc2(&f), "KA9Q-1>APDR15:/test info");
    }

    #[test]
    fn via_with_h_bit_gets_star() {
        let f = make_frame(
            "KA9Q-1",
            "APDR15",
            vec![
                ("WIDE1-1".to_string(), false),
                ("KD9PDP-3".to_string(), true),
            ],
            b"!data",
        );
        assert_eq!(to_tnc2(&f), "KA9Q-1>APDR15,WIDE1-1,KD9PDP-3*:!data");
    }

    #[test]
    fn all_via_heard() {
        let f = make_frame(
            "W9XYZ",
            "APRS",
            vec![
                ("RELAY".to_string(), true),
                ("WIDE".to_string(), true),
            ],
            b">status",
        );
        assert_eq!(to_tnc2(&f), "W9XYZ>APRS,RELAY*,WIDE*:>status");
    }

    #[test]
    fn ssid_preserved_in_output() {
        let f = make_frame("N0CALL-9", "APDW16", vec![], b"=position");
        assert_eq!(to_tnc2(&f), "N0CALL-9>APDW16:=position");
    }
}
