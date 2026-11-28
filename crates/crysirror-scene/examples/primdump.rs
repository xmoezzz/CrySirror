// Temporary diagnostic: flatten a layer command dump and print primitives.
use crysirror_core::state::ClientId;
use crysirror_scene::layer::{LayerCommand, LayerSystem};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cmds: Vec<LayerCommand> = serde_json::from_slice(&std::fs::read(&a[1]).unwrap()).unwrap();
    let (lo, hi): (u32, u32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let mut s = LayerSystem::default();
    s.apply_batch(ClientId(1), &cmds).unwrap();
    for p in s.frame([800, 600]).unwrap() {
        let (px, py) = (467.5f32, 300.5f32);
        let qx0 = p.quad.iter().map(|q| q[0]).fold(f32::MAX, f32::min);
        let qx1 = p.quad.iter().map(|q| q[0]).fold(f32::MIN, f32::max);
        let qy0 = p.quad.iter().map(|q| q[1]).fold(f32::MAX, f32::min);
        let qy1 = p.quad.iter().map(|q| q[1]).fold(f32::MIN, f32::max);
        let inq = px >= qx0 && px < qx1 && py >= qy0 && py < qy1;
        let inc = p.clip.is_none_or(|c| px >= c[0] && px < c[0] + c[2] && py >= c[1] && py < c[1] + c[3]);
        if (lo..=hi).contains(&p.id) || (lo == 0 && inq && inc) {
            println!("{:?}", std::mem::discriminant(&p.paint));
            println!("  id {} op {} quad {:?} clip {:?} uv {:?}", p.id, p.opacity, p.quad.map(|q| [q[0], q[1]]), p.clip, p.uv);
        }
    }
}
