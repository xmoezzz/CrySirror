// SPDX-License-Identifier: MPL-2.0
//! Self-authored native scene bytes, built from the pinned interpreter's ABI.
//! No user game or font bytes. This is not the CrySirror data VM format.
use crate::runtime::{
    constants::{cd, fm, op},
    forms::codes,
};
#[derive(Default)]
pub struct SceneBuilder {
    pub code: Vec<u8>,
    pub labels: Vec<i32>,
    pub strings: Vec<String>,
}
impl SceneBuilder {
    pub fn i32(&mut self, v: i32) {
        self.code.extend(v.to_le_bytes());
    }
    pub fn int(&mut self, v: i32) {
        self.code.push(cd::PUSH);
        self.i32(fm::INT);
        self.i32(v);
    }
    pub fn element(&mut self, chain: &[i32]) {
        self.code.push(cd::ELM_POINT);
        for v in chain {
            self.int(*v);
        }
    }
    pub fn command(&mut self, chain: &[i32], overload: i32, args: &[i32], ret: i32) {
        self.element(chain);
        for v in args {
            self.int(*v);
        }
        self.code.push(cd::COMMAND);
        self.i32(overload);
        self.i32(args.len() as i32);
        for _ in args {
            self.i32(fm::INT);
        }
        self.i32(0);
        self.i32(ret);
    }

    pub fn string(&mut self, text: &str) {
        let id = self.strings.len();
        self.strings.push(text.into());
        self.code.push(cd::PUSH);
        self.i32(fm::STR);
        self.i32(id as i32);
    }
    pub fn command_string(
        &mut self,
        chain: &[i32],
        overload: i32,
        text: &str,
        args: &[i32],
        ret: i32,
    ) {
        self.element(chain);
        self.string(text);
        for v in args {
            self.int(*v);
        }
        self.code.push(cd::COMMAND);
        self.i32(overload);
        self.i32(args.len() as i32 + 1);
        for _ in args {
            self.i32(fm::INT);
        }
        self.i32(fm::STR);
        self.i32(0);
        self.i32(ret);
    }
    pub fn finish(self) -> Vec<u8> {
        let labels = 132 + self.code.len();
        let z = labels + self.labels.len() * 4;
        let indexes = z + 4;
        let strings = indexes + self.strings.len() * 8;
        let mut text = Vec::new();
        let mut index = Vec::new();
        for s in &self.strings {
            let words: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
            index.extend((text.len() as i32 / 2).to_le_bytes());
            index.extend((words.len() as i32).to_le_bytes());
            for w in words {
                text.extend(w.to_le_bytes());
            }
        }
        let end = strings + text.len();
        let mut h = [0i32; 33];
        for i in [3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31] {
            h[i] = end as i32;
        }
        h[0] = 132;
        h[1] = 132;
        h[2] = self.code.len() as i32;
        h[3] = indexes as i32;
        h[4] = self.strings.len() as i32;
        h[5] = strings as i32;
        h[6] = self.strings.len() as i32;
        h[7] = labels as i32;
        h[8] = self.labels.len() as i32;
        h[9] = z as i32;
        h[10] = 1;
        let mut out = Vec::new();
        for w in h {
            out.extend(w.to_le_bytes());
        }
        out.extend(self.code);
        for l in self.labels {
            out.extend(l.to_le_bytes());
        }
        out.extend(0i32.to_le_bytes());
        out.extend(index);
        out.extend(text);
        out
    }
}
pub fn demo_scene() -> Vec<u8> {
    let mut s = SceneBuilder::default();
    s.command(
        &[codes::ELM_GLOBAL_FRONT, codes::ELM_STAGE_CREATE_OBJECT],
        0,
        &[2],
        fm::VOID,
    );
    s.command(
        &[
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_CREATE_RECT,
        ],
        2,
        &[0, 0, 180, 100, 35, 180, 220, 255, 1, 28, 28],
        fm::VOID,
    );
    s.int(6);
    s.int(7);
    s.code.push(cd::OPERATE_2);
    s.i32(fm::INT);
    s.i32(fm::INT);
    s.code.push(op::MULTIPLE);
    s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[100], fm::VOID);
    s.command(
        &[
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_X,
        ],
        1,
        &[180],
        fm::VOID,
    );
    s.code.push(cd::EOF);
    s.finish()
}
/// An uncompressed, plain-string Scene.pck in the original pack ABI.
pub fn pack_scene(scene: &[u8]) -> Vec<u8> {
    let name: Vec<u16> = "entry".encode_utf16().collect();
    let names = 92 + 8;
    let index = names + name.len() * 2;
    let data = index + 8;
    let mut h = [0i32; 23];
    h[0] = 92;
    h[13] = 92;
    h[14] = 1;
    h[15] = names as i32;
    h[16] = 1;
    h[17] = index as i32;
    h[18] = 1;
    h[19] = data as i32;
    h[20] = 1;
    let mut out = Vec::new();
    for n in h {
        out.extend(n.to_le_bytes());
    }
    out.extend(0i32.to_le_bytes());
    out.extend((name.len() as i32).to_le_bytes());
    for c in name {
        out.extend(c.to_le_bytes());
    }
    out.extend(0i32.to_le_bytes());
    out.extend((scene.len() as i32).to_le_bytes());
    out.extend(scene);
    out
}

pub fn image_scene() -> Vec<u8> {
    let mut s = SceneBuilder::default();
    s.command(
        &[codes::ELM_GLOBAL_FRONT, codes::ELM_STAGE_CREATE_OBJECT],
        0,
        &[1],
        fm::VOID,
    );
    s.command_string(
        &[
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_CREATE,
        ],
        2,
        "native",
        &[1, 20, 20],
        fm::VOID,
    );
    s.command(
        &[
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_SCALE_X,
        ],
        1,
        &[120000],
        fm::VOID,
    );
    s.command(
        &[
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_SCALE_Y,
        ],
        1,
        &[80000],
        fm::VOID,
    );
    s.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[100], fm::VOID);
    s.code.push(cd::EOF);
    s.finish()
}
pub fn pixel_g00() -> Vec<u8> {
    let mut b = vec![0, 1, 0, 1, 0];
    b.extend(0u32.to_le_bytes());
    b.extend(4u32.to_le_bytes());
    b.extend([1, 10, 200, 20]);
    b
}

/// Native TIMEWAIT_KEY fixture, with a real SAVEPOINT when requested. A held
/// DECIDE key does not complete the wait; the original down/up edge does.
pub fn key_wait_scene(savepoint: bool) -> Vec<u8> {
    let mut s = SceneBuilder::default();
    s.command(
        &[codes::ELM_GLOBAL_FRONT, codes::ELM_STAGE_CREATE_OBJECT],
        0,
        &[1],
        fm::VOID,
    );
    let object = [
        codes::ELM_GLOBAL_FRONT,
        codes::ELM_STAGE_OBJECT,
        codes::ELM_ARRAY,
        0,
    ];
    s.command(
        &[object.as_slice(), &[codes::ELM_OBJECT_CREATE_RECT]].concat(),
        2,
        &[0, 0, 100, 60, 40, 180, 220, 255, 1, 28, 28],
        fm::VOID,
    );
    if savepoint {
        s.command(&[codes::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    }
    s.command(&[codes::ELM_GLOBAL_TIMEWAIT_KEY], 0, &[5000], fm::INT);
    s.command(
        &[object.as_slice(), &[codes::ELM_OBJECT_X]].concat(),
        1,
        &[180],
        fm::VOID,
    );
    s.int(123);
    s.code.push(cd::EOF);
    s.finish()
}

/// A real CD_TEXT instruction, intentionally useful for distinguishing engine
/// text capabilities from completion of the native message-window bridge.
pub fn message_scene() -> Vec<u8> {
    let mut s = SceneBuilder::default();
    s.string("Native Siglus message: 原生文字");
    s.code.push(cd::TEXT);
    s.i32(-1);
    s.code.push(cd::EOF);
    s.finish()
}

/// Plain native NAME/TEXT chunks. The adapter maps them to retained glyphs;
/// this is compiled native scene data rather than data-VM draw commands.
pub fn dialogue_scene() -> Vec<u8> {
    let mut scene = SceneBuilder::default();
    scene.string("CrySirror");
    scene.code.push(cd::NAME);
    for text in ["Hello, CrySirror.\n", "原生メッセージ"] {
        scene.string(text);
        scene.code.push(cd::TEXT);
        scene.i32(-1);
    }
    scene.code.push(cd::EOF);
    scene.finish()
}
