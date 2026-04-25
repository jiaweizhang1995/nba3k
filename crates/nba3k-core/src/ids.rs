use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($name:ident, $inner:ty) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub $inner);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<$inner> for $name {
            fn from(v: $inner) -> Self {
                Self(v)
            }
        }
    };
}

id_newtype!(PlayerId, u32);
id_newtype!(TeamId, u8);
id_newtype!(SeasonId, u16);
id_newtype!(GameId, u64);
id_newtype!(TradeId, u64);
id_newtype!(DraftPickId, u32);
