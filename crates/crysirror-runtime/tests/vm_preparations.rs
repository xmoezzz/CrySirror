//! Offline startup polls do not replay peers, consume input or advance clocks.
use crysirror_core::{service::{HostServices,InputFrame},state::ClientId,vm::{VmAdapter,VmYield}};
use crysirror_runtime::{AdapterRegistry,Runtime,Project,model::ClientSpec,runtime::VmFactory};
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
struct Factory {name:&'static str,boot:u8,calls:Arc<AtomicUsize>,pressed:Arc<AtomicUsize>}
struct Port {name:&'static str,boot:u8,calls:Arc<AtomicUsize>,pressed:Arc<AtomicUsize>}
impl VmFactory for Factory {
    fn create(&self,_client:ClientId,_spec:&ClientSpec)->anyhow::Result<Box<dyn VmAdapter>> {
        Ok(Box::new(Port{name:self.name,boot:self.boot,calls:self.calls.clone(),pressed:self.pressed.clone()}))
    }
}
impl VmAdapter for Port {
    fn preparation_pending(&self)->bool{self.boot>0}
    fn engine_id(&self)->&'static str{self.name}
    fn snapshot_schema(&self)->u32{1}
    fn step(&mut self,_host:&mut dyn HostServices,input:InputFrame,_budget:u32)->crysirror_core::Result<VmYield> {
        self.calls.fetch_add(1,Ordering::Relaxed);
        if self.boot>0 {assert_eq!(input.buttons_pressed,0);self.boot-=1;} else {self.pressed.store(input.buttons_pressed as usize,Ordering::Relaxed);}
        Ok(VmYield::BudgetExhausted)
    }
    fn snapshot(&self)->crysirror_core::Result<Vec<u8>>{Err(crysirror_core::Error::Unsupported("authored snapshot"))}
    fn restore(&mut self,_bytes:&[u8])->crysirror_core::Result<()>{Err(crysirror_core::Error::Unsupported("authored restore"))}
}
#[test]
fn preparation_fence_polls_only_starting_vms_and_preserves_first_normal_frame_input() {
    let root=tempfile::tempdir().unwrap();
    let project:Project=serde_json::from_value(serde_json::json!({"schema":2,"name":"startup","content_revision":"1","assets":"game","canvas":[4,4],"clients":[{"name":"boot","adapter":"test-boot","game":"boot"},{"name":"peer","adapter":"test-peer","game":"peer"}]})).unwrap();
    let boot_calls=Arc::new(AtomicUsize::new(0));let peer_calls=Arc::new(AtomicUsize::new(0));let boot_input=Arc::new(AtomicUsize::new(0));let peer_input=Arc::new(AtomicUsize::new(0));
    let mut registry=AdapterRegistry::default();
    registry.register("test-boot",Arc::new(Factory{name:"test-boot",boot:2,calls:boot_calls.clone(),pressed:boot_input.clone()})).unwrap();
    registry.register("test-peer",Arc::new(Factory{name:"test-peer",boot:0,calls:peer_calls.clone(),pressed:peer_input.clone()})).unwrap();
    let mut rt=Runtime::with_asset_root(project,root.path(),&root.path().join("state"),registry).unwrap();
    assert!(rt.poll_vm_preparations().unwrap());assert!(!rt.poll_vm_preparations().unwrap());assert!(!rt.poll_vm_preparations().unwrap());
    assert_eq!(boot_calls.load(Ordering::Relaxed),2);assert_eq!(peer_calls.load(Ordering::Relaxed),0);
    assert_eq!(rt.host.scene.frame(),0);assert_eq!(rt.inspect()["time_ms"],0);
    rt.tick(16,InputFrame{buttons_pressed:1,..Default::default()}).unwrap();
    assert_eq!(boot_calls.load(Ordering::Relaxed),3);assert_eq!(peer_calls.load(Ordering::Relaxed),1);
    assert_eq!(boot_input.load(Ordering::Relaxed),1);assert_eq!(peer_input.load(Ordering::Relaxed),1);
    assert_eq!(rt.host.scene.frame(),1);assert_eq!(rt.inspect()["time_ms"],16);
}
