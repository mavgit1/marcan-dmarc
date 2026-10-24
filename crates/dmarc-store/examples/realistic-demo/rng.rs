//! A small seeded generator (`SplitMix64`), so a seed always gives the same
//! database.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5eed_0fd3_a002_0260)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    /// `x` rounded up or down at random, so small daily rates still add up.
    pub fn round(&mut self, x: f64) -> i64 {
        let x = x.max(0.0);
        let floor = x.floor();
        floor as i64 + i64::from(self.f64() < x - floor)
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// An index drawn by `weights` (not all zero).
    pub fn weighted(&mut self, weights: &[f64]) -> usize {
        let total: f64 = weights.iter().sum();
        let mut x = self.f64() * total;
        for (i, w) in weights.iter().enumerate() {
            if x < *w {
                return i;
            }
            x -= w;
        }
        weights.len() - 1
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }

    /// `count` distinct items of `items` (all of them when there are fewer).
    pub fn sample<T: Clone>(&mut self, items: &[T], count: usize) -> Vec<T> {
        let mut all = items.to_vec();
        self.shuffle(&mut all);
        all.truncate(count);
        all
    }

    /// Lowercase hex, `len` digits.
    pub fn hex(&mut self, len: usize) -> String {
        (0..len)
            .map(|_| char::from_digit(self.below(16) as u32, 16).unwrap_or('0'))
            .collect()
    }

    /// Decimal digits, `len` of them, not starting with 0.
    pub fn digits(&mut self, len: usize) -> String {
        (0..len)
            .map(|i| {
                let d = if i == 0 {
                    1 + self.below(9)
                } else {
                    self.below(10)
                };
                char::from_digit(d as u32, 10).unwrap_or('1')
            })
            .collect()
    }
}
