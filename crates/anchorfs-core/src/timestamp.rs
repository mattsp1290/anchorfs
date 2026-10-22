//! RFC3339 serde boundary for the shared time vocabulary.
use serde::{Deserialize, Deserializer, Serializer};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub fn serialize<S: Serializer>(value: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.format(&Rfc3339).map_err(serde::ser::Error::custom)?)
}
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<OffsetDateTime, D::Error> {
    OffsetDateTime::parse(&String::deserialize(deserializer)?, &Rfc3339)
        .map_err(serde::de::Error::custom)
}
pub mod option {
    use super::*;
    pub fn serialize<S: Serializer>(
        value: &Option<OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer
                .serialize_some(&value.format(&Rfc3339).map_err(serde::ser::Error::custom)?),
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| OffsetDateTime::parse(&text, &Rfc3339).map_err(serde::de::Error::custom))
            .transpose()
    }
}
