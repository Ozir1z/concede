//! Record identifiers.
//!
//! Every record is identified by a ULID: 128 bits, time-ordered, encoded as 26 Crockford
//! base32 characters. Each record kind gets its own newtype so the compiler stops us from
//! passing a capture id where a decision id is expected (ADR 0005).

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::SystemTime;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use ulid::{Generator, Ulid};

/// Process-wide generator so ids minted in the same millisecond still sort in creation
/// order. Ordering across processes falls back to the random part, which is fine: events
/// from different processes in the same millisecond have no meaningful order anyway.
static GENERATOR: Mutex<Generator> = Mutex::new(Generator::new());

/// Mint a new ULID for the given instant.
pub(crate) fn new_ulid(at: SystemTime) -> Ulid {
    // A poisoned mutex only means another thread panicked mid-generate; the generator
    // state is still a valid ULID, so keep using it.
    let mut generator = GENERATOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    generator
        .generate_from_datetime(at)
        .unwrap_or_else(|_overflow| Ulid::from_datetime(at))
}

/// Error returned when a string is not a valid ULID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a valid id (expected a 26-character ULID): {0:?}")]
pub struct ParseIdError(pub String);

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            /// Mint a new id for the given instant.
            pub fn new_at(at: SystemTime) -> Self {
                Self(new_ulid(at))
            }

            /// The underlying ULID.
            pub fn ulid(&self) -> Ulid {
                self.0
            }
        }

        impl From<Ulid> for $name {
            fn from(ulid: Ulid) -> Self {
                Self(ulid)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_string(s)
                    .map(Self)
                    .map_err(|_| ParseIdError(s.to_owned()))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "description": "ULID: 26 Crockford base32 characters, time-ordered.",
                    "pattern": "^[0-9A-HJKMNP-TV-Z]{26}$"
                })
            }
        }
    };
}

define_id!(
    /// Identifies a decision record (`.concede/decisions/<id>.json`).
    DecisionId
);
define_id!(
    /// Identifies an event record (`.concede/events/<id>.json`).
    EventId
);
define_id!(
    /// Identifies a hook capture (`.concede/captures/<id>.json`).
    CaptureId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_minted_in_one_instant_sort_in_creation_order() {
        let now = SystemTime::now();
        let ids: Vec<DecisionId> = (0..100).map(|_| DecisionId::new_at(now)).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn id_round_trips_through_string_and_json() {
        let id = EventId::new_at(SystemTime::now());
        let text = id.to_string();
        assert_eq!(text.len(), 26);
        assert_eq!(text.parse::<EventId>().unwrap(), id);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<EventId>(&json).unwrap(), id);
    }

    #[test]
    fn rejects_malformed_id() {
        assert!("not-a-ulid".parse::<CaptureId>().is_err());
        assert!(serde_json::from_str::<CaptureId>("\"short\"").is_err());
    }
}
