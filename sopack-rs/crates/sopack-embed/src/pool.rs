//! Pooling + normalisation — the last step from `[batch, seq, dim]` encoder
//! output to one vector per text. Mirrors the M0 spike's `embed_batch`
//! (`spike/src/main.rs`) exactly (mean pooling accumulates in `f64`, same as
//! numpy promoting `f32 * int64 mask` internally), generalised only to also
//! support `cls` pooling — see `spec::Pooling`'s docs for why that exists.

use crate::spec::Pooling;
use ndarray::{Array2, ArrayView3};

/// Pools `hidden` (`[batch, seq, dim]`, the encoder's `last_hidden_state`)
/// down to `batch` vectors of length `dim`, using `mask` (`[batch, seq]`,
/// 1 for a real token / 0 for padding) for `Pooling::Mean`, then
/// L2-normalises each vector if `normalize`. Accumulates in `f64` before
/// casting back to `f32`, matching numpy's implicit promotion in the
/// Python reference this was measured against (M0: cosine ≥ 0.99999999998
/// against 281 stored vectors).
pub fn pool_and_normalize(
    hidden: &ArrayView3<f32>,
    mask: &Array2<i64>,
    pooling: Pooling,
    normalize: bool,
) -> Vec<Vec<f32>> {
    let (b, l, d) = hidden.dim();
    let mut out = Vec::with_capacity(b);
    for i in 0..b {
        let mut v: Vec<f64> = match pooling {
            Pooling::Mean => mean_pool_one(hidden, mask, i, l, d),
            Pooling::Cls => (0..d).map(|k| hidden[[i, 0, k]] as f64).collect(),
        };
        if normalize {
            l2_normalize_f64(&mut v);
        }
        out.push(v.into_iter().map(|x| x as f32).collect());
    }
    out
}

fn mean_pool_one(
    hidden: &ArrayView3<f32>,
    mask: &Array2<i64>,
    i: usize,
    l: usize,
    d: usize,
) -> Vec<f64> {
    let mut acc = vec![0f64; d];
    let mut n = 0f64;
    for j in 0..l {
        if mask[[i, j]] != 0 {
            n += 1.0;
            for k in 0..d {
                acc[k] += hidden[[i, j, k]] as f64;
            }
        }
    }
    let n = n.max(1e-9);
    for a in acc.iter_mut() {
        *a /= n;
    }
    acc
}

fn l2_normalize_f64(v: &mut [f64]) {
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
    for x in v.iter_mut() {
        *x /= norm;
    }
}

/// L2-normalises an `f32` vector in place — used where the values are
/// already `f32` (e.g. the calibration fixture's stored vectors) and
/// double-precision accumulation isn't in play.
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt()
        .max(1e-12);
    for x in v.iter_mut() {
        *x = (*x as f64 / norm) as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array3;

    /// batch=2, seq=3, dim=2. Text 0 has all 3 positions real; text 1 has
    /// only the first 2 real (position 2 is padding and must be masked
    /// out of the mean).
    fn fixture() -> (Array3<f32>, Array2<i64>) {
        let hidden = Array3::from_shape_vec(
            (2, 3, 2),
            vec![
                1.0, 1.0, // text0 pos0
                3.0, 3.0, // text0 pos1
                5.0, 5.0, // text0 pos2
                2.0, 4.0, // text1 pos0
                6.0, 8.0, // text1 pos1
                999.0, 999.0, // text1 pos2 (padding — must be ignored)
            ],
        )
        .unwrap();
        let mask = Array2::from_shape_vec((2, 3), vec![1, 1, 1, 1, 1, 0]).unwrap();
        (hidden, mask)
    }

    #[test]
    fn mean_pooling_averages_only_unmasked_positions() {
        let (hidden, mask) = fixture();
        let out = pool_and_normalize(&hidden.view(), &mask, Pooling::Mean, false);
        // text0: mean of (1,1),(3,3),(5,5) = (3,3)
        assert!((out[0][0] - 3.0).abs() < 1e-6);
        assert!((out[0][1] - 3.0).abs() < 1e-6);
        // text1: mean of (2,4),(6,8) = (4,6) — the 999 padding row must be excluded
        assert!((out[1][0] - 4.0).abs() < 1e-6);
        assert!((out[1][1] - 6.0).abs() < 1e-6);
    }

    #[test]
    fn cls_pooling_takes_position_zero_regardless_of_mask() {
        let (hidden, mask) = fixture();
        let out = pool_and_normalize(&hidden.view(), &mask, Pooling::Cls, false);
        assert_eq!(out[0], vec![1.0, 1.0]);
        assert_eq!(out[1], vec![2.0, 4.0]);
    }

    #[test]
    fn normalize_produces_unit_vectors() {
        let (hidden, mask) = fixture();
        let out = pool_and_normalize(&hidden.view(), &mask, Pooling::Mean, true);
        for v in &out {
            let norm: f64 = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "expected unit norm, got {norm}");
        }
    }

    #[test]
    fn mean_and_cls_disagree_when_cls_is_not_the_mean() {
        // A fixture where the CLS-token direction and the mean-of-tokens
        // direction are not collinear (unlike `fixture()`, whose rows are
        // all scalar multiples of `(1, 1)`) — needed so the two pooling
        // strategies actually produce different unit vectors.
        let hidden = Array3::from_shape_vec(
            (1, 3, 2),
            vec![
                1.0, 0.0, // pos0 (CLS)
                0.0, 1.0, // pos1
                0.0, 1.0, // pos2
            ],
        )
        .unwrap();
        let mask = Array2::from_shape_vec((1, 3), vec![1, 1, 1]).unwrap();
        let mean = pool_and_normalize(&hidden.view(), &mask, Pooling::Mean, true);
        let cls = pool_and_normalize(&hidden.view(), &mask, Pooling::Cls, true);
        // This is the exact property the calibration gate leans on: wrong
        // pooling produces a materially different vector.
        let dot: f64 = mean[0]
            .iter()
            .zip(&cls[0])
            .map(|(a, b)| (*a as f64) * (*b as f64))
            .sum();
        assert!(
            dot < 0.999,
            "mean and cls pooling should diverge on this fixture, cosine={dot}"
        );
    }

    #[test]
    fn l2_normalize_in_place_produces_unit_vector() {
        let mut v = vec![3.0f32, 4.0f32];
        l2_normalize(&mut v);
        let norm = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
        assert!((v[0] - 0.6).abs() < 1e-5);
        assert!((v[1] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn l2_normalize_does_not_divide_by_zero_on_a_zero_vector() {
        let mut v = vec![0.0f32, 0.0f32];
        l2_normalize(&mut v);
        assert!(v.iter().all(|x| x.is_finite()));
    }
}
