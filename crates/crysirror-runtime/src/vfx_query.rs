//! Client-scoped reads of completed VFX worker receipts. Declarations and
//! admitted tasks cannot manufacture a ready state or random-sequence value.
use crate::runtime::SharedHost;
use anyhow::{Context, Result, ensure};
use crysirror_core::state::ClientId;
use serde_json::{Value, json};

impl SharedHost {
    pub(crate) fn query_vfx_sequence(
        &self,
        client: ClientId,
        entity: u32,
        revision: u64,
        completed_tick: u64,
        exchange_epoch: u64,
    ) -> Result<Value> {
        let (_, generation, entity_spec) = self.engine.world.entities(client)
            .into_iter().find(|(id, _, _)| *id == entity)
            .context("VFX entity not found in client scope")?;
        let system = entity_spec.vfx.as_ref().context("entity has no modular VFX system")?;
        ensure!(system.revision == revision, "VFX sequence query revision is stale");
        // This optional authored field is the opt-in contract. Receipt values
        // below always come from worker completion, never this declaration.
        ensure!(system.sequence_rng.is_some(), "VFX system has no shared sequence RNG");
        let fingerprint = blake3::hash(&serde_json::to_vec(system)?).to_hex().to_string();
        let row = self.vfx_sequences.get(&(client.0, entity)).filter(|row| {
            row["entity_generation"].as_u64() == Some(u64::from(generation))
                && row["revision"].as_u64() == Some(revision)
                && row["source_fingerprint"].as_str() == Some(fingerprint.as_str())
        });
        if let Some(row) = row {
            if row["phase"] == "failed" {
                return Ok(row.clone());
            }
            if row["phase"] == "ready"
                && row["completed_tick"].as_u64().is_some_and(|v| v >= completed_tick)
                && row["exchange_epoch"].as_u64().is_some_and(|v| v >= exchange_epoch)
            {
                ensure!(!row["sequence_rng"].is_null(), "completed VFX sequence receipt has no state");
                return Ok(row.clone());
            }
        }
        Ok(json!({
            "phase": "pending", "entity_generation": generation, "revision": revision,
            "requested_tick": completed_tick, "requested_exchange_epoch": exchange_epoch,
            "completed_tick": row.and_then(|v| v.get("completed_tick")),
            "exchange_epoch": row.and_then(|v| v.get("exchange_epoch")),
            "sequence_rng": null,
        }))
    }
}
