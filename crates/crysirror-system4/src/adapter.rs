// Copyright (C) 2026 CrySirror contributors. GPL-2.0-or-later.
//! Thin native syscall policy over CrySirror's shared clock. No factory is
//! registered until resource bootstrap and useful presentation are implemented.
//! The optional Windows debug-log interface is explicitly absent; this does not
//! implement or pretend to buffer, persist, or connect a generic log service.
use crate::vm::{
    ExternalCall, ExternalTarget, Machine,
    external::{Argument, HllReturn},
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::service::HostServices;
use sha2::{Digest, Sha256};
/// Verified original OutputLog.dll (2004-03-10), never loaded or executed.
#[derive(Clone, Debug)]
pub struct AbsentDebugLog {
    _verified: (),
}
impl AbsentDebugLog {
    pub fn verify_native_library(bytes: &[u8]) -> Result<Self> {
        const HASH: [u8; 32] = [
            0x8c, 0x02, 0x3f, 0x38, 0x77, 0x69, 0x94, 0xdb, 0x5c, 0x7c, 0xc3, 0x6a, 0x5c, 0xe4,
            0x09, 0xb4, 0x33, 0xef, 0xc2, 0x51, 0xa8, 0x79, 0xfc, 0xf8, 0x32, 0xb6, 0x07, 0xb6,
            0x2f, 0xee, 0x04, 0xc8,
        ];
        ensure!(
            bytes.len() == 33280 && Sha256::digest(bytes)[..] == HASH,
            "unreviewed System4 OutputLog DLL ABI"
        );
        Ok(Self { _verified: () })
    }
}
#[derive(Clone, Debug, Default)]
pub struct Bridge {
    pub debug_log: Option<AbsentDebugLog>,
    sleep: Option<(u32, u64)>,
    pub absent_debug_calls: u64,
    pub clock_calls: u64,
    pub log_calls: u64,
    pub sact: Option<crate::sact::Sact>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Completed,
    WaitUntil(u64),
}
impl Bridge {
    pub fn dispatch(
        &mut self,
        m: &mut Machine,
        call: &ExternalCall,
        host: &mut dyn HostServices,
    ) -> Result<Dispatch> {
        self.dispatch_scoped(m,call,host,None)
    }
    pub fn dispatch_for(&mut self,client:crysirror_core::state::ClientId,m:&mut Machine,call:&ExternalCall,host:&mut dyn HostServices)->Result<Dispatch> {
        self.dispatch_scoped(m,call,host,Some(client))
    }
    fn dispatch_scoped(&mut self,m:&mut Machine,call:&ExternalCall,host:&mut dyn HostServices,client:Option<crysirror_core::state::ClientId>)->Result<Dispatch> {
        match call.target {
            ExternalTarget::Hll { library, function } => {
                let l = &m.program.libraries[library];
                let f = &l.functions[function];
                let args = m.hll_arguments(call)?;
                if l.name == b"SACT2" && args.is_empty() && f.return_kind==10 {
                    let value=match f.name.as_slice() {
                        b"GetScreenWidth"=>Some(self.sact.as_ref().context("SACT2 native canvas absent")?.canvas[0] as i32),
                        b"GetScreenHeight"=>Some(self.sact.as_ref().context("SACT2 native canvas absent")?.canvas[1] as i32),
                        b"GetMainSurfaceNumber"=>Some(-1),
                        b"GAME_MSG_GetNumof"=>Some(i32::try_from(m.program.messages.len())?),
                        b"Timer_Get"=>{self.clock_calls+=1;Some(host.now_ms() as u32 as i32)},
                        _=>None,
                    };
                    if let Some(value)=value {m.complete_hll(call,HllReturn::Scalar(value))?;return Ok(Dispatch::Completed);}
                }
                if l.name == b"SACT2" && f.name == b"Init" {
                    ensure!(f.return_kind==10 && args.len()==2 && matches!(args[1],Argument::Scalar{kind:10,..}),"SACT2.Init ABI");
                    let client=client.context("SACT2.Init requires explicit client scope")?;
                    let sact=self.sact.as_mut().context("SACT2.Init requires native logical canvas metadata")?;
                    let result=sact.init(client,host)?;
                    m.complete_hll(call,HllReturn::Scalar(result))?;
                    return Ok(Dispatch::Completed);
                }
                if l.name == b"OutputLog" && self.debug_log.is_some() {
                    // Original DLL Create RVA1000 tries GetModuleHandleA(DPLogViewer.ade),
                    // GetProcAddress(GetInterface), GUID, then vtable[0]. Without that module
                    // the exact native branch returns -1. Other disconnected exports have
                    // their own explicit behavior. No generic unknown-HLL no-op fallback.
                    let result = match (f.name.as_slice(), f.return_kind, args.as_slice()) {
                        (b"Create", 10, [Argument::String { .. }]) => HllReturn::Scalar(-1),
                        (
                            b"Output",
                            0,
                            [Argument::Scalar { kind: 10, .. }, Argument::String { .. }],
                        ) => HllReturn::Void,
                        (b"Clear", 0, [Argument::Scalar { kind: 10, .. }]) => HllReturn::Void,
                        (
                            b"Save",
                            10 | 47,
                            [Argument::Scalar { kind: 10, .. }, Argument::String { .. }],
                        ) => HllReturn::Scalar(0),
                        (
                            b"EnableAutoSave",
                            47,
                            [Argument::Scalar { kind: 10, .. }, Argument::String { .. }],
                        ) => HllReturn::Scalar(0),
                        (b"DisableAutoSave", 47, [Argument::Scalar { kind: 10, .. }]) => {
                            HllReturn::Scalar(0)
                        }
                        _ => bail!("unsupported reviewed OutputLog function/ABI"),
                    };
                    m.complete_hll(call, result)?;
                    self.absent_debug_calls += 1;
                    return Ok(Dispatch::Completed);
                }
                bail!(
                    "unsupported original System4 HLL {}.{} pc={:#x}; external call retained",
                    String::from_utf8_lossy(&l.name),
                    String::from_utf8_lossy(&f.name),
                    call.pc
                )
            }
            ExternalTarget::System(6) => {
                let client=client.context("System4 output requires explicit client scope")?;
                m.validate_system(call)?;
                let bytes=m.heap.string(m.stack_value(0)?)?;
                let text=encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(bytes.as_slice()).context("System4 system.Output encoding")?;
                host.append_log(client,&text)?;
                // system.Output returns the same native string: the caller's
                // following S_POP remains responsible for releasing it.
                m.complete_system(call,0,None)?;
                self.log_calls+=1;
                Ok(Dispatch::Completed)
            }
            ExternalTarget::System(0x0d) => {
                let now = host.now_ms() as u32 as i32;
                m.complete_system(call, 0, Some(now))?;
                self.clock_calls += 1;
                Ok(Dispatch::Completed)
            }
            ExternalTarget::System(0x15) => {
                let ms = m.stack_value(0)?;
                ensure!(ms >= 0, "System4 negative Sleep unsupported");
                let now = host.now_ms();
                let deadline = if let Some((pc, end)) = self.sleep {
                    ensure!(pc == call.pc, "stale System4 sleep continuation");
                    end
                } else {
                    let end = now
                        .checked_add(ms as u64)
                        .context("System4 sleep clock overflow")?;
                    self.sleep = Some((call.pc, end));
                    end
                };
                if now < deadline {
                    return Ok(Dispatch::WaitUntil(deadline));
                }
                m.complete_system(call, 1, None)?;
                self.sleep = None;
                self.clock_calls += 1;
                Ok(Dispatch::Completed)
            }
            _ => bail!(
                "unsupported original System4 syscall {:?} pc={:#x}; external call retained",
                call.target,
                call.pc
            ),
        }
    }
}
