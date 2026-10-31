// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Portable serde transport for the device-free Core input buffer. Embeddings
//! share this schema instead of implementing private input replay algorithms.
use crysirror_core::{input_buffer::*, service::InputFrame};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
#[derive(Serialize, Deserialize)]
#[serde(remote = "InputFrame", deny_unknown_fields)]
pub struct FrameWire {
    #[serde(with = "exact_u64")]
    pub buttons_down: u64,
    #[serde(with = "exact_u64")]
    pub buttons_pressed: u64,
    pub pointer_x: i32,
    pub pointer_y: i32,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "InputSample", deny_unknown_fields)]
pub struct SampleWire {
    #[serde(with = "exact_u64")]
    pub sequence: u64,
    #[serde(with = "exact_u64")]
    pub time_ms: u64,
    #[serde(with = "FrameWire")]
    pub frame: InputFrame,
    #[serde(with = "exact_u64")]
    pub released: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "InputReceipt", deny_unknown_fields)]
pub struct ReceiptWire {
    pub client: u32,
    #[serde(with = "exact_u64")]
    pub epoch: u64,
    #[serde(with = "exact_u64")]
    pub sequence: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sample(#[serde(with = "SampleWire")] pub InputSample);
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct Receipt(#[serde(with = "ReceiptWire")] pub InputReceipt);
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct Frame(#[serde(with = "FrameWire")] InputFrame);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub capacity: usize,
    #[serde(with = "exact_u64")]
    pub next_sequence: u64,
    #[serde(with = "exact_u64")]
    pub consumed_sequence: u64,
    #[serde(with = "exact_u64")]
    pub base_down: u64,
    observed: Option<Frame>,
    #[serde(with = "exact_u64")]
    pub observed_time: u64,
    #[serde(with = "exact_u64")]
    pub leased_through: u64,
    pub samples: Vec<Sample>,
}
impl From<InputBufferSnapshot> for Snapshot {
    fn from(s: InputBufferSnapshot) -> Self {
        Self {
            schema: s.schema,
            capacity: s.capacity,
            next_sequence: s.next_sequence,
            consumed_sequence: s.consumed_sequence,
            base_down: s.base_down,
            observed: s.observed.map(Frame),
            observed_time: s.observed_time,
            leased_through: s.leased_through,
            samples: s.samples.into_iter().map(Sample).collect(),
        }
    }
}
impl From<Snapshot> for InputBufferSnapshot {
    fn from(s: Snapshot) -> Self {
        Self {
            schema: s.schema,
            capacity: s.capacity,
            next_sequence: s.next_sequence,
            consumed_sequence: s.consumed_sequence,
            base_down: s.base_down,
            observed: s.observed.map(|f| f.0),
            observed_time: s.observed_time,
            leased_through: s.leased_through,
            samples: s.samples.into_iter().map(|f| f.0).collect(),
        }
    }
}
/// Engine-library state, serializable for logical continuations. No device or
/// task handle is retained. Core owns normalization, admission and receipts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputInbox(pub InputBuffer);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedInbox {
    client: u32,
    #[serde(with = "exact_u64")]
    epoch: u64,
    state: Snapshot,
}
impl Serialize for InputInbox {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        SavedInbox {
            client: self.0.client(),
            epoch: self.0.epoch(),
            state: self.0.snapshot().into(),
        }
        .serialize(s)
    }
}
impl<'de> Deserialize<'de> for InputInbox {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = SavedInbox::deserialize(d)?;
        InputBuffer::restore(s.client, s.epoch, s.state.into())
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

/// Human-readable transports use decimal strings for exact 64-bit values.
/// Binary snapshots keep native u64 encoding. Small numeric input is accepted;
/// large JS numbers are rejected rather than silently losing low button bits.
pub(crate) mod exact_u64 {
    use serde::{
        Deserializer, Serializer,
        de::{Error, Visitor},
    };
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str(&v.to_string())
        } else {
            s.serialize_u64(*v)
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        if !d.is_human_readable() {
            return serde::Deserialize::deserialize(d);
        }
        struct Exact;
        impl<'de> Visitor<'de> for Exact {
            type Value = u64;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a decimal u64 string or safe nonnegative integer")
            }
            fn visit_str<E: Error>(self, s: &str) -> Result<u64, E> {
                s.parse().map_err(E::custom)
            }
            fn visit_u64<E: Error>(self, v: u64) -> Result<u64, E> {
                if v > 9007199254740991 {
                    Err(E::custom("large input integer requires a decimal string"))
                } else {
                    Ok(v)
                }
            }
            fn visit_i64<E: Error>(self, v: i64) -> Result<u64, E> {
                if v < 0 {
                    Err(E::custom("negative input integer"))
                } else {
                    self.visit_u64(v as u64)
                }
            }
            fn visit_f64<E: Error>(self, v: f64) -> Result<u64, E> {
                if v.is_finite() && v >= 0. && v.fract() == 0. && v <= 9007199254740991. {
                    Ok(v as u64)
                } else {
                    Err(E::custom("input integer requires a decimal string"))
                }
            }
        }
        d.deserialize_any(Exact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn human_wire_keeps_high_button_bits_exact_and_rejects_unsafe_js_numbers() {
        let mask = (1u64 << 60) | 1;
        let mut q = InputBuffer::new(1, 7, 4).unwrap();
        q.push(
            1,
            3,
            InputFrame {
                buttons_down: mask,
                ..Default::default()
            },
        )
        .unwrap();
        let inbox = InputInbox(q);
        let json = serde_json::to_value(&inbox).unwrap();
        assert_eq!(
            json["state"]["samples"][0]["frame"]["buttons_pressed"],
            mask.to_string()
        );
        assert_eq!(
            serde_json::from_value::<InputInbox>(json.clone()).unwrap(),
            inbox
        );
        let mut bad = json;
        bad["state"]["observed"]["buttons_down"] = serde_json::json!(mask);
        assert!(serde_json::from_value::<InputInbox>(bad).is_err());
    }
}
