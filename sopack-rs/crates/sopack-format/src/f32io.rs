//! Little-endian float32 (de)serialization for `vectors.f32`/`probe.f32` —
//! the Rust equivalent of `sopack.format._f32_bytes`/`_f32_read`.

pub const ITEM_SIZE: usize = 4;

/// One vector as little-endian float32 bytes.
pub fn f32_to_le_bytes(vector: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(vector.len() * ITEM_SIZE);
    for v in vector {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

/// A run of little-endian float32 bytes back into vectors of *dim*. *raw*'s
/// length must be a multiple of `dim * 4`; any remainder is dropped (callers
/// are expected to have already checked exact lengths — see
/// [`crate::reader::PackReader`]).
pub fn le_bytes_to_f32_vectors(raw: &[u8], dim: usize) -> Vec<Vec<f32>> {
    let stride = dim * ITEM_SIZE;
    if stride == 0 {
        return Vec::new();
    }
    raw.chunks_exact(stride)
        .map(|chunk| {
            chunk
                .chunks_exact(ITEM_SIZE)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let vectors = vec![vec![1.0f32, -2.5, 0.0], vec![3.25, 42.0, -0.0]];
        let mut raw = Vec::new();
        for v in &vectors {
            raw.extend_from_slice(&f32_to_le_bytes(v));
        }
        let got = le_bytes_to_f32_vectors(&raw, 3);
        assert_eq!(got, vectors);
    }

    #[test]
    fn matches_known_le_encoding() {
        // 1.0f32 = 0x3F800000 -> LE bytes 00 00 80 3F
        assert_eq!(f32_to_le_bytes(&[1.0]), vec![0x00, 0x00, 0x80, 0x3F]);
    }
}
