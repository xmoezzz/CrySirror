// SPDX-License-Identifier: MPL-2.0
use crysirror_asset::{
    anim,
    pbr::{PbrModel, SceneNode, Trs},
};
use std::sync::Arc;
#[test]
fn parallel_pose_results_match_serial_and_preserve_instance_order() {
    let pool = crysirror_tasks::TaskPool::new("poses", 3, 16).unwrap();
    let requests: Vec<_> = (0..37)
        .map(|i| {
            let mut model = PbrModel::default();
            model.nodes.push(SceneNode {
                name: "root".into(),
                children: vec![],
                mesh: None,
                skin: None,
                rest: Trs {
                    translation: glam::Vec3::new(i as f32, 2., 3.),
                    rotation: glam::Quat::IDENTITY,
                    scale: glam::Vec3::ONE,
                },
            });
            model.roots.push(0);
            model.animations.push(crysirror_asset::pbr::Clip {
                name: "move".into(),
                duration: 1.,
                channels: vec![crysirror_asset::pbr::Channel {
                    node: 0,
                    path: crysirror_asset::pbr::ChannelPath::Translation,
                    interpolation: crysirror_asset::pbr::Interpolation::Linear,
                    times: vec![0., 1.],
                    values: vec![[0., 0., 0., 0.], [i as f32, 1., 0., 0.]],
                }],
            });
            (
                Arc::new(model),
                vec![anim::Layer {
                    clip: 0,
                    time: i as f32 / 37.,
                    weight: 1.,
                }],
            )
        })
        .collect();
    let result = anim::poses_parallel(&pool, &requests).unwrap();
    for ((model, layers), pose) in requests.iter().zip(result) {
        assert_eq!(anim::pose(model, layers), pose);
    }
    assert_eq!(pool.stats().completed, 37);
}
