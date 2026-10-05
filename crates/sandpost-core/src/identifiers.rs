//! Opaque domain identifiers and the numeric message sequence.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! opaque_identifier {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            /// Generate a fresh random UUID identifier.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            /// Create a fresh identifier using the same random generation as `new`.
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $name {
            /// Write the identifier in the canonical UUID display format.
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            /// Parse a UUID string and preserve the parser error for invalid input.
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}
opaque_identifier!(MessageIdentifier);
opaque_identifier!(ScopeIdentifier);
opaque_identifier!(UserIdentifier);
opaque_identifier!(InboxIdentifier);
opaque_identifier!(EndpointIdentifier);
opaque_identifier!(ViewIdentifier);

/// Internal SQLite AUTOINCREMENT key; opaque UUIDs are used in public APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageSequence(pub u64);
