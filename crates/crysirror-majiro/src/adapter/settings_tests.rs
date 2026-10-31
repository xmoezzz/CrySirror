use super::*;
use crysirror_core::service::{ClientStorage, FileSystem, StorageMetadata};
use std::sync::Mutex;
struct Control {
    status: StorageWriteState,
    writes: usize,
    bytes: Vec<u8>,
}
struct Receipt(Arc<Mutex<Control>>);
impl StorageWrite for Receipt {
    fn poll(&mut self) -> crysirror_core::Result<StorageWriteState> {
        Ok(self.0.lock().unwrap().status.clone())
    }
}
struct Storage(Arc<Mutex<Control>>);
impl ClientStorage for Storage {
    fn metadata(&self, _: Mount, _: &str) -> crysirror_core::Result<Option<StorageMetadata>> {
        Ok(None)
    }
    fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::NotFound)
    }
    fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        panic!("native must use shared asynchronous storage actor")
    }
    fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
    fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
        Ok(vec![])
    }
    fn begin_write_atomic(
        &self,
        path: &VfsPath,
        bytes: Arc<[u8]>,
    ) -> crysirror_core::Result<Box<dyn StorageWrite>> {
        assert_eq!(path.mount(), Mount::Save);
        assert_eq!(path.relative(), SETTINGS_FILE);
        let mut c = self.0.lock().unwrap();
        c.writes += 1;
        c.bytes = bytes.to_vec();
        Ok(Box::new(Receipt(self.0.clone())))
    }
}
struct Host(Arc<Storage>);
impl FileSystem for Host {
    fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::NotFound)
    }
    fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
}
impl HostServices for Host {
    fn submit(
        &mut self,
        _: ClientId,
        _: &[crysirror_core::state::Command],
    ) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("unused"))
    }
    fn now_ms(&self) -> u64 {
        0
    }
    fn storage(&mut self, client: ClientId) -> crysirror_core::Result<Arc<dyn ClientStorage>> {
        assert_eq!(client, ClientId(1));
        Ok(self.0.clone())
    }
}
fn setup() -> (MajiroVm, Host, Arc<Mutex<Control>>) {
    let c = Arc::new(Mutex::new(Control {
        status: StorageWriteState::Pending,
        writes: 0,
        bytes: vec![],
    }));
    let storage = Arc::new(Storage(c.clone()));
    let mut vm = MajiroVm::new(
        ClientId(1),
        serde_json::from_value(json!({"script":"start.mjo"})).unwrap(),
    )
    .unwrap();
    let code = [
        0x00, 0x08, 0x56, 0x01, 0, 0, 0x35, 0x08, 0xb4, 0xfc, 0x93, 0x0c, 1, 0, 0, 0x08, 0, 0, 0,
        0, 0x2b, 0x08,
    ];
    let mut module = b"MajiroObjV1.000\0".to_vec();
    for n in [0, 0, 1, 0x12345678, 0, code.len() as u32] {
        module.extend(n.to_le_bytes());
    }
    module.extend(code);
    vm.machine = Some(Machine::new(Arc::new(Program::parse(&module).unwrap())));
    vm.native_abi = true;
    vm.settings = Some(
        NativeSettings::load(
            b"REG_GLOBAL maker\nREG_LOCAL game\nEFFECTSPEED 725\n",
            storage.as_ref(),
        )
        .unwrap(),
    );
    (vm, Host(storage), c)
}
#[test]
fn native_settings_wait_for_real_shared_write_receipt_without_repeating_or_acknowledging() {
    let (mut vm, mut h, c) = setup();
    assert_eq!(vm.run(&mut h, 100).unwrap(), VmYield::WaitUntil(1));
    assert_eq!(vm.machine.as_ref().unwrap().pc, 6);
    assert_eq!(vm.settings.as_ref().unwrap().effect_speed, 725);
    assert_eq!(vm.run(&mut h, 100).unwrap(), VmYield::WaitUntil(1));
    assert_eq!(c.lock().unwrap().writes, 1);
    assert!(vm.poll_settings_write(999).is_err());
    assert_eq!(vm.settings.as_ref().unwrap().effect_speed, 725);
    c.lock().unwrap().status = StorageWriteState::Ready;
    assert_eq!(vm.run(&mut h, 1).unwrap(), VmYield::BudgetExhausted);
    assert_eq!(vm.machine.as_ref().unwrap().pc, 14);
    assert_eq!(vm.settings.as_ref().unwrap().effect_speed, 342);
    assert_eq!(vm.settings.as_ref().unwrap().commits, 1);
    assert!(vm.settings_write.is_none());
    let saved: serde_json::Value = serde_json::from_slice(&c.lock().unwrap().bytes).unwrap();
    assert_eq!(saved["values"]["effectspeed"], "342");
}
#[test]
fn failed_shared_settings_write_keeps_native_pc_and_previous_published_values() {
    let (mut vm, mut h, c) = setup();
    vm.run(&mut h, 100).unwrap();
    c.lock().unwrap().status = StorageWriteState::Failed("disk failure".into());
    let error = vm.run(&mut h, 100).unwrap_err();
    assert!(format!("{error:#}").contains("settings write failed"));
    assert_eq!(vm.machine.as_ref().unwrap().pc, 6);
    assert_eq!(
        vm.machine.as_ref().unwrap().stack,
        vec![Value::Integer(342)]
    );
    assert_eq!(vm.settings.as_ref().unwrap().effect_speed, 725);
    assert_eq!(vm.settings.as_ref().unwrap().commits, 0);
    assert_eq!(c.lock().unwrap().writes, 1);
}
