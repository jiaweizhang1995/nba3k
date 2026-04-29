use serde::{Deserialize, Serialize};

/// Money in integer cents — never use f64 for money.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct Cents(pub i64);

impl Cents {
    pub const ZERO: Self = Self(0);

    pub fn from_dollars(d: i64) -> Self {
        Self(d.saturating_mul(100))
    }

    pub fn as_dollars(self) -> i64 {
        self.0 / 100
    }

    pub fn as_millions_f32(self) -> f32 {
        self.0 as f32 / 100.0 / 1_000_000.0
    }
}

impl std::ops::Add for Cents {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

impl std::ops::Sub for Cents {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

impl std::ops::AddAssign for Cents {
    fn add_assign(&mut self, other: Self) {
        self.0 = self.0.saturating_add(other.0);
    }
}

impl std::iter::Sum for Cents {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |a, b| a + b)
    }
}

impl std::fmt::Display for Cents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${:.2}M", self.as_millions_f32())
    }
}
