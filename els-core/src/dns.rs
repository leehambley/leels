//! Captive-portal DNS: answers every A query with the device's own address,
//! so any name a phone looks up leads to the setup page.

/// Build a response to `query` in `out`; `None` if it isn't a usable query.
pub fn answer(query: &[u8], ip: [u8; 4], out: &mut [u8]) -> Option<usize> {
    if query.len() < 12 || query[2] & 0x80 != 0 {
        return None; // too short, or itself a response
    }
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if qdcount != 1 {
        return None;
    }
    // Walk the question name (labels, no compression in queries).
    let mut i = 12;
    loop {
        let len = *query.get(i)? as usize;
        if len == 0 {
            i += 1;
            break;
        }
        if len & 0xC0 != 0 {
            return None;
        }
        i += 1 + len;
    }
    let question_end = i + 4;
    let qtype = u16::from_be_bytes([*query.get(i)?, *query.get(i + 1)?]);
    let qclass = u16::from_be_bytes([*query.get(i + 2)?, *query.get(i + 3)?]);
    let is_a = qtype == 1 && qclass == 1;

    let total = question_end + if is_a { 16 } else { 0 };
    let out = out.get_mut(..total)?;
    out[..question_end].copy_from_slice(&query[..question_end]);
    // Response, opcode copied, authoritative, recursion desired copied, recursion available.
    out[2] = 0x84 | (query[2] & 0x79);
    out[3] = 0x80;
    out[4..12].copy_from_slice(&[0, 1, 0, is_a as u8, 0, 0, 0, 0]);
    if is_a {
        let a = &mut out[question_end..];
        a[..2].copy_from_slice(&[0xC0, 0x0C]); // name: pointer to the question
        a[2..4].copy_from_slice(&1u16.to_be_bytes()); // type A
        a[4..6].copy_from_slice(&1u16.to_be_bytes()); // class IN
        a[6..10].copy_from_slice(&60u32.to_be_bytes()); // TTL
        a[10..12].copy_from_slice(&4u16.to_be_bytes());
        a[12..16].copy_from_slice(&ip);
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Query for "example.com" type A, id 0x1234, recursion desired.
    const QUERY: &[u8] = &[
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 7, b'e', b'x', b'a', b'm', b'p', b'l',
        b'e', 3, b'c', b'o', b'm', 0, 0x00, 0x01, 0x00, 0x01,
    ];

    #[test]
    fn answers_a_query_with_our_ip() {
        let mut out = [0u8; 512];
        let n = answer(QUERY, [192, 168, 4, 1], &mut out).unwrap();
        assert_eq!(n, QUERY.len() + 16);
        assert_eq!(&out[..2], &[0x12, 0x34]);
        assert_eq!(out[2] & 0x80, 0x80, "response flag");
        assert_eq!(&out[6..8], &[0, 1], "one answer");
        assert_eq!(&out[n - 4..n], &[192, 168, 4, 1]);
    }

    #[test]
    fn aaaa_gets_empty_answer() {
        let mut q = QUERY.to_vec();
        let t = q.len() - 3;
        q[t] = 28; // AAAA
        let mut out = [0u8; 512];
        let n = answer(&q, [192, 168, 4, 1], &mut out).unwrap();
        assert_eq!(n, q.len());
        assert_eq!(&out[6..8], &[0, 0]);
    }

    #[test]
    fn ignores_garbage_and_responses() {
        let mut out = [0u8; 512];
        assert_eq!(answer(&[1, 2, 3], [0; 4], &mut out), None);
        let mut resp = QUERY.to_vec();
        resp[2] |= 0x80;
        assert_eq!(answer(&resp, [0; 4], &mut out), None);
        assert_eq!(answer(&QUERY[..20], [0; 4], &mut out), None);
    }
}
