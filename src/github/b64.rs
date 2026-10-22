//! Standard base64, as the contents API sends it: padded, wrapped at 60 characters.

/// Decode `s`, skipping whitespace. `None` on any other non-alphabet byte or bad padding.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits, mut pad) = (0u32, 0u32, 0u32);
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        if pad > 0 {
            return None; // data after padding
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // 6 leftover bits cannot form a byte; padding must match what is missing.
    let ok = match bits {
        0 => pad == 0,
        4 => pad == 0 || pad == 2,
        2 => pad == 0 || pad == 1,
        _ => false,
    };
    ok.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(decode(enc).unwrap(), plain.as_bytes(), "{enc}");
        }
        assert_eq!(decode("Zg").unwrap(), b"f"); // unpadded
    }

    #[test]
    fn github_wraps_at_60_characters_with_newlines() {
        let plain = "The quick brown fox jumps over the lazy dog. ".repeat(4);
        let flat = "VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZy4g\
VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZy4g\
VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZy4g\
VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZy4g";
        let wrapped: String = flat
            .as_bytes()
            .chunks(60)
            .map(|c| format!("{}\n", std::str::from_utf8(c).unwrap()))
            .collect();
        assert_eq!(wrapped.lines().next().unwrap().len(), 60);
        assert_eq!(decode(&wrapped).unwrap(), plain.as_bytes());
        assert_eq!(decode(flat).unwrap(), plain.as_bytes());
    }

    #[test]
    fn non_ascii_and_binary_bytes_survive() {
        assert_eq!(decode("w7xuw68=").unwrap(), "ünï".as_bytes());
        assert_eq!(decode("/+8A").unwrap(), [0xff, 0xef, 0x00]);
    }

    #[test]
    fn bad_input_is_none() {
        assert_eq!(decode("Zm9v!"), None);
        assert_eq!(decode("Zg=A"), None);
        assert_eq!(decode("Z"), None);
        assert_eq!(decode("Zm9vY==="), None);
    }
}
