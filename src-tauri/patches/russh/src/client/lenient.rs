//! Lenient decoding of server key material, opt-in through
//! `Config::lenient_host_key_encoding`.
//!
//! RFC 4251 §5 requires `mpint` values to be minimally encoded, and
//! `ssh-encoding` rejects anything else. Legacy SSH servers — switches,
//! routers and firewalls with old firmware — pad positive integers with
//! redundant leading zero bytes anyway, which used to fail the whole
//! handshake with `` `mpint` encoding invalid ``. The functions here strip
//! exactly that padding, never touching anything else: blobs whose algorithm
//! does not frame its parts as integers are returned unchanged, and a blob
//! that cannot be parsed as the expected shape is returned unchanged too, so
//! the strict decoder keeps reporting the real problem.

/// Host key algorithms whose blob is `[string name][mpint…]`: RSA keys carry
/// the exponent and modulus, DSA keys the four DSA parameters. Every other
/// algorithm (ed25519, ecdsa, …) frames its parts as plain strings, where a
/// leading zero is data and must survive untouched.
const MPINT_SHAPED_HOST_KEYS: &[&[u8]] = &[
    b"ssh-rsa",
    b"rsa-sha2-256",
    b"rsa-sha2-512",
    b"ssh-dss",
];

/// Signature algorithms whose blob is `[string name][string sig]` with `sig`
/// holding integers: a raw RSA value, or the two `r`/`s` halves ECDSA signs
/// with. DSA is deliberately absent — RFC 4253 §6.6 frames a DSA signature
/// as one raw 40-byte value with no length prefixes inside, so there is
/// nothing to normalize without breaking the frame.
fn is_mpint_shaped_signature(algorithm: &[u8]) -> bool {
    algorithm == b"ssh-rsa" || algorithm.starts_with(b"ecdsa-sha2-")
}

/// Splits off one length-prefixed field, returning it and what follows.
fn split_field(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let (&length, rest) = input.split_first()?;
    let length = u32::from_be_bytes([length, rest.first().copied()?, rest.get(1).copied()?, rest.get(2).copied()?]) as usize;
    rest.get(3..)?.split_at_checked(length)
}

/// The minimal encoding of one integer: strips padding a positive value does
/// not need. Zero, which RFC 4251 encodes as an empty value, is the one
/// single-byte case.
fn minimal_mpint(field: &[u8]) -> Vec<u8> {
    let mut value = field;
    while let [0x00, rest @ ..] = value {
        match rest.first() {
            // A real sign byte: the value is positive with the MSB set.
            Some(next) if *next >= 0x80 => break,
            Some(_) => value = rest,
            None => break,
        }
    }
    if value == [0x00] {
        value = &[];
    }
    value.to_vec()
}

/// Re-encodes a run of length-prefixed fields minimally. `None` when `input`
/// is not exactly a sequence of well-formed fields.
fn minimal_fields(mut input: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut fields = Vec::new();
    while !input.is_empty() {
        let (field, rest) = split_field(input)?;
        fields.push(minimal_mpint(field));
        input = rest;
    }
    Some(fields)
}

fn encode_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&u32::try_from(field.len()).expect("field fits in u32").to_be_bytes());
    out.extend_from_slice(field);
}

fn encode_fields(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        encode_field(&mut out, field);
    }
    out
}

/// A host key blob with every over-padded integer minimized; the algorithm
/// name and any unrecognized shape come back untouched.
pub(crate) fn normalize_host_key_blob(blob: &[u8]) -> Vec<u8> {
    let Some((algorithm, rest)) = split_field(blob) else {
        return blob.to_vec();
    };
    if !MPINT_SHAPED_HOST_KEYS.contains(&algorithm) {
        return blob.to_vec();
    }
    let Some(fields) = minimal_fields(rest) else {
        return blob.to_vec();
    };
    let mut normalized = Vec::with_capacity(blob.len());
    encode_field(&mut normalized, algorithm);
    for field in &fields {
        encode_field(&mut normalized, field);
    }
    normalized
}

/// A signature blob with every over-padded integer minimized; anything but
/// the integer-framed shapes comes back untouched.
pub(crate) fn normalize_signature_blob(blob: &[u8]) -> Vec<u8> {
    let Some((algorithm, rest)) = split_field(blob) else {
        return blob.to_vec();
    };
    let Some((data, trailer)) = split_field(rest) else {
        return blob.to_vec();
    };
    if !trailer.is_empty() || !is_mpint_shaped_signature(algorithm) {
        return blob.to_vec();
    }
    // RSA signs with one raw integer; ECDSA wraps two length-prefixed halves
    // inside the signature string.
    let data = if algorithm == b"ssh-rsa" {
        minimal_mpint(data)
    } else {
        match minimal_fields(data) {
            Some(fields) => encode_fields(&fields),
            None => return blob.to_vec(),
        }
    };
    let mut normalized = Vec::with_capacity(blob.len());
    encode_field(&mut normalized, algorithm);
    encode_field(&mut normalized, &data);
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        encode_field(&mut out, value);
        out
    }

    #[test]
    fn mpint_padding_is_stripped() {
        assert_eq!(minimal_mpint(&[0x00, 0x03]), [0x03]);
        assert_eq!(minimal_mpint(&[0x00, 0x00]), []);
        assert_eq!(minimal_mpint(&[0x00]), []);
        assert_eq!(minimal_mpint(&[0x00, 0x00, 0x7f]), [0x7f]);
        // A leading zero on a value with the MSB set is the sign byte.
        assert_eq!(minimal_mpint(&[0x00, 0x80]), [0x00, 0x80]);
        assert_eq!(minimal_mpint(&[0x01, 0x00, 0x01]), [0x01, 0x00, 0x01]);
    }

    #[test]
    fn host_key_blob_keeps_non_integer_algorithms() {
        // ed25519's key is one string: a leading zero there is data.
        let blob = [&field(b"ssh-ed25519")[..], &field(&[0x00, 0x7f, 0x01])[..]].concat();
        assert_eq!(normalize_host_key_blob(&blob), blob);
    }

    #[test]
    fn host_key_blob_is_minimized() {
        let blob = [
            &field(b"ssh-rsa")[..],
            &field(&[0x00, 0x03])[..],
            &field(&[0x00, 0x80, 0xff])[..],
        ]
        .concat();
        assert_eq!(
            normalize_host_key_blob(&blob),
            [&field(b"ssh-rsa")[..], &field(&[0x03])[..], &field(&[0x00, 0x80, 0xff])[..]].concat()
        );
    }

    #[test]
    fn host_key_blob_survives_malformed_input() {
        for blob in [
            &field(b"ssh-rsa")[..b"ssh-rsa".len() - 1], // truncated name
            &field(b"ssh-rsa")[..],                     // no fields at all
            &{
                let mut truncated = field(b"ssh-rsa");
                truncated.extend_from_slice(&[0, 0, 0, 9, 1]); // short field
                truncated
            },
        ] {
            assert_eq!(normalize_host_key_blob(blob), blob);
        }
    }

    #[test]
    fn signature_blob_is_minimized() {
        let rsa = [&field(b"ssh-rsa")[..], &field(&[0x00, 0x42])[..]].concat();
        assert_eq!(
            normalize_signature_blob(&rsa),
            [&field(b"ssh-rsa")[..], &field(&[0x42])[..]].concat()
        );
        // [string alg][string sig], sig = [mpint r][mpint s]
        let ecdsa = [
            &field(b"ecdsa-sha2-nistp256")[..],
            &field(&[&field(&[0x00, 0x01])[..], &field(&[0xff])[..]].concat())[..],
        ]
        .concat();
        assert_eq!(
            normalize_signature_blob(&ecdsa),
            [
                &field(b"ecdsa-sha2-nistp256")[..],
                &field(&[&field(&[0x01])[..], &field(&[0xff])[..]].concat())[..],
            ]
            .concat()
        );
    }

    #[test]
    fn signature_blob_keeps_trailing_data() {
        let blob = [&field(b"ssh-rsa")[..], &field(&[0x00, 0x42])[..], &[9][..]].concat();
        assert_eq!(normalize_signature_blob(&blob), blob);
    }

    #[test]
    fn signature_blob_keeps_the_dss_frame() {
        // RFC 4253 §6.6: a DSA signature is one raw 40-byte value, no
        // length-prefixed halves inside — nothing to normalize.
        let blob = [&field(b"ssh-dss")[..], &field(&[0xab; 40])[..]].concat();
        assert_eq!(normalize_signature_blob(&blob), blob);
    }
}
