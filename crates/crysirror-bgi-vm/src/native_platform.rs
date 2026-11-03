//! Native platform request policy. No OS, shell, registry or filesystem access.
use crate::{SysApi, Vm, VmError, VmResult};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NativePlatformRequest {
    ClipboardSet {
        text: String,
    },
    UserName,
    ComputerName,
    SpecialFolder {
        mode: i32,
    },
    FileDialog {
        initial_directory: String,
        description: String,
        extension: String,
        title: String,
        mode: i32,
    },
    ResourceFileDialog {
        mode: i32,
        title: String,
        default_name: String,
        filters: Vec<(String, String)>,
    },
    FolderDialog {
        title: String,
        root: i32,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativePlatformResponse {
    ClipboardSet(bool),
    Text(String),
    SpecialFolder(Option<String>),
    FileDialog(Result<Option<String>, i32>),
    FolderDialog(Option<String>),
}
impl Vm {
    /// Preflight without changing the guest operand ring. The original native
    /// handler still owns its validation/status and BP output-buffer writes.
    pub(crate) fn prepare_native_platform<A: SysApi>(
        &mut self,
        api: &mut A,
        code: u8,
        id: u8,
    ) -> VmResult<bool> {
        if !api.host_requests_enabled() {
            return Ok(true);
        }
        let text = |vm: &Self, tail| -> VmResult<String> {
            let at = vm.operand_from_top(tail).ok_or(VmError::StackUnderflow)?;
            vm.value_as_string_lossy(vm.operand_value(at))
        };
        let int = |vm: &Self, tail| -> VmResult<i32> {
            let at = vm.operand_from_top(tail).ok_or(VmError::StackUnderflow)?;
            Ok(vm.operand_value(at).as_i32())
        };
        if matches!((code, id), (0x80, 0x3b) | (0x81, 0x38)) && !matches!(int(self, 0)?, 0 | 1) {
            return Ok(true);
        }
        let request = match (code, id) {
            (0x7e, _) => NativePlatformRequest::ClipboardSet {
                text: text(self, 0)?,
            },
            (0x81, 0x08) => NativePlatformRequest::UserName,
            (0x81, 0x09) => NativePlatformRequest::ComputerName,
            (0x80, 0x3a) => NativePlatformRequest::SpecialFolder {
                mode: int(self, 0)?,
            },
            (0x80, 0x3b) => NativePlatformRequest::FileDialog {
                mode: int(self, 0)?,
                initial_directory: text(self, 1)?,
                title: text(self, 2)?,
                extension: text(self, 3)?,
                description: text(self, 4)?,
            },
            (0x81, 0x3a) => NativePlatformRequest::FolderDialog {
                root: int(self, 0)?,
                title: text(self, 1)?,
            },
            (0x81, 0x38) => {
                let count = int(self, 5)?;
                if count <= 0 {
                    return Ok(true);
                }
                if count > 256 {
                    return Err(VmError::Runtime("native file-dialog filter budget".into()));
                }
                let labels = int(self, 4)? as u32;
                let extensions = int(self, 3)? as u32;
                let mut filters = Vec::with_capacity(count as usize);
                for i in 0..count as u32 {
                    let label = self.read_int(labels.wrapping_add(i * 4), 2)?;
                    let extension = self.read_int(extensions.wrapping_add(i * 4), 2)?;
                    filters.push((self.read_c_string(label)?, self.read_c_string(extension)?));
                }
                NativePlatformRequest::ResourceFileDialog {
                    mode: int(self, 0)?,
                    title: text(self, 1)?,
                    default_name: text(self, 2)?,
                    filters,
                }
            }
            _ => return Ok(true),
        };
        api.begin_native_storage(self.thread.thread_id() as u32 as u64);
        api.prepare_native_platform(&request)
    }
}
