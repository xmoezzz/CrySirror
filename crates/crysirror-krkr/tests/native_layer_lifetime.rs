// Distributed under the GNU Affero General Public License, version 3 or later.
//! Native tree bookkeeping is non-owning; script references and active work own it.
use crysirror_core::{image_processing::{ImageInput,ImageResource},state::ClientId};
use crysirror_krkr::{host::KrkrHost,natives};
use crysirror_tjs2::runtime::{Runtime,Variant};
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};

fn script(rt:&mut Runtime<KrkrHost>,source:&str) {
    natives::run_source(rt,"authored-lifetime.tjs",source,None).unwrap();
}
fn setup()->Runtime<KrkrHost> {
    let mut rt=Runtime::with_host(KrkrHost::new([32,32],"default".into()));
    natives::install(&mut rt);
    script(&mut rt,"var w=new Window(),root=new Layer(w,null),keep=new Layer(w,root),temporary=new Layer(w,root);");
    rt
}

#[test]
fn renderer_tree_does_not_permanently_root_unreferenced_native_layer() {
    let mut rt=setup();
    let h=rt.global_member("temporary").object_handle().unwrap();
    let slot=rt.host().layers[&h.0].slot;
    script(&mut rt,"temporary=null;");
    rt.collect_garbage(&[]);
    assert!(!rt.object_valid(h));
    assert!(!rt.host().layers.contains_key(&h.0));
    assert_eq!(rt.host().layers.len(),2);
    script(&mut rt,"var replacement=new Layer(w,root);");
    let replacement=rt.global_member("replacement").object_handle().unwrap();
    assert_eq!(rt.host().layers[&replacement.0].slot,slot);
}

#[test]
fn collecting_parent_detaches_but_does_not_collect_script_owned_children() {
    let mut rt=setup();
    let parent=rt.global_member("root").object_handle().unwrap();
    let child=rt.global_member("keep").object_handle().unwrap();
    script(&mut rt,"root=null;temporary=null;");
    rt.collect_garbage(&[]);
    assert!(!rt.object_valid(parent));
    assert!(rt.object_valid(child));
    assert_eq!(rt.host().layers[&child.0].parent,None);
    assert!(!rt.host().layers[&child.0].attached);
}

#[derive(Debug)]
struct TrackedImage(Arc<AtomicUsize>);
impl Drop for TrackedImage {fn drop(&mut self){self.0.fetch_add(1,Ordering::SeqCst);}}
impl ImageResource for TrackedImage {
    fn client(&self)->ClientId{ClientId(1)}
    fn size(&self)->[u32;2]{[1,1]}
    fn rgba(&self)->Arc<[u8]>{Arc::from([255u8;4])}
    fn opaque(&self)->bool{true}
}

#[test]
fn active_work_roots_then_retirement_releases_only_the_native_image_lease() {
    let mut rt=setup();
    let h=rt.global_member("temporary").object_handle().unwrap();
    let drops=Arc::new(AtomicUsize::new(0));
    let outside:Arc<dyn ImageResource>=Arc::new(TrackedImage(drops.clone()));
    rt.host_mut().layer_mut(h).unwrap().image_source=Some(ImageInput::Resource(outside.clone()));
    script(&mut rt,"temporary=null;");
    rt.collect_garbage(&[Variant::Object(h)]);
    assert!(rt.object_valid(h));
    assert!(rt.host().layers.contains_key(&h.0));
    rt.collect_garbage(&[]);
    assert!(!rt.object_valid(h));
    assert!(!rt.host().layers.contains_key(&h.0));
    assert_eq!(drops.load(Ordering::SeqCst),0,"external immutable image lease must survive");
    drop(outside);
    assert_eq!(drops.load(Ordering::SeqCst),1);
}
