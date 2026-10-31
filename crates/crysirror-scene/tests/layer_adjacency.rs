// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Repeated retained edits preserve hierarchy through failed batches/restores.
use crysirror_core::state::ClientId;
use crysirror_scene::layer::{LayerCommand as C, LayerSystem};
fn put(id: u32, parent: Option<u32>) -> C {
    serde_json::from_value(serde_json::json!({"op":"put","layer":{
        "id":id,"parent":parent,"rect":[0,0,1,1],"paint":{"kind":"solid","color":[0,0,0,255]}
    }}))
    .unwrap()
}
fn seed() -> LayerSystem {
    let mut s = LayerSystem::default();
    s.apply_batch(
        ClientId(1),
        &[
            put(1, None),
            put(2, Some(1)),
            put(3, Some(2)),
            put(4, None),
            put(5, None),
        ],
    )
    .unwrap();
    s.apply_batch(ClientId(1), &[C::Remove { id: 3 }]).unwrap();
    s.apply_batch(ClientId(1), &[put(3, Some(2)), put(7, Some(3))])
        .unwrap();
    s
}
#[test]
fn reparented_descendants_follow_the_latest_parent_across_batches_and_clients() {
    let mut s = seed();
    let c = ClientId(1);
    s.apply_batch(ClientId(2), &[put(1, None), put(3, Some(1))])
        .unwrap();
    s.apply_batch(c, &[put(3, Some(4))]).unwrap();
    s.apply_batch(c, &[C::Remove { id: 1 }]).unwrap();
    assert!(s.get(c, 2).is_none());
    assert!(s.get(c, 3).is_some());
    assert!(s.get(c, 7).is_some());
    s.apply_batch(c, &[C::Remove { id: 4 }]).unwrap();
    assert!(s.get(c, 3).is_none());
    assert!(s.get(c, 7).is_none());
    assert!(s.get(ClientId(2), 3).is_some());
}
#[test]
fn failed_batch_and_nested_transaction_restore_descendant_membership() {
    let mut s = seed();
    let c = ClientId(1);
    assert!(
        s.apply_batch(c, &[put(3, Some(7))]).is_err(),
        "cycle must fail atomically"
    );
    let before = serde_json::to_value(&s).unwrap();
    let result: anyhow::Result<()> = s.transaction(|s| {
        s.apply_batch(c, &[put(3, Some(5))])?;
        s.transaction(|s| s.apply_batch(c, &[C::Remove { id: 5 }]))?;
        anyhow::bail!("authored rollback")
    });
    assert!(result.is_err());
    assert_eq!(serde_json::to_value(&s).unwrap(), before);
    s.apply_batch(c, &[C::Remove { id: 2 }]).unwrap();
    assert!(s.get(c, 3).is_none());
    assert!(s.get(c, 7).is_none());
    assert!(s.get(c, 5).is_some());
}
#[test]
fn clones_and_deserialized_views_rebuild_independent_indexes() {
    let mut original = seed();
    let c = ClientId(1);
    let mut clone = original.clone();
    let mut restored: LayerSystem =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    clone
        .apply_batch(c, &[put(3, Some(4)), C::Remove { id: 2 }])
        .unwrap();
    assert!(clone.get(c, 7).is_some());
    original.apply_batch(c, &[C::Remove { id: 2 }]).unwrap();
    restored.apply_batch(c, &[C::Remove { id: 2 }]).unwrap();
    assert!(original.get(c, 7).is_none());
    assert!(restored.get(c, 7).is_none());
    clone.apply_batch(c, &[C::Remove { id: 4 }]).unwrap();
    assert!(clone.get(c, 7).is_none());
}
