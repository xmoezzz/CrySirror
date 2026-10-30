//! Self-authored native BP fixture. No game/font bytes are distributed.
use crysirror_bgi_vm::Builder;
pub fn scene(text: bool, audio: bool, input: bool) -> Vec<u8> {
    let mut b = Builder::default();
    for v in [0, 320, 180, 1] {
        b.int(v);
    }
    b.call(0x90, 0x11);
    b.int(0).int(0x18304f).call(0x90, 0x13);
    b.int(0).call(0x90, 0x40);
    b.int(1).string("").string("pixel.png").call(0x90, 0x10);
    b.call(0x90, 0x50);
    // The first native sprite slot is 0x80000000; the returned handle is also
    // retained on the operand ring, matching native unused-result behaviour.
    for v in [i32::MIN, 250, 120, 1, 128, 0, 10] {
        b.int(v);
    }
    b.call(0x90, 0x56);
    if text {
        b.int(0).int(10).int(16).string("CrySirror BGI");
        for v in [
            0xffffff, 0, 0, 0, 0, 20, 100, 0, 0, 0, 100, 0, 0, 0, 0, 0, 0,
        ] {
            b.int(v);
        }
        b.call(0x92, 0x9c);
    }
    if audio {
        b.int(0).string("tone.wav").int(128).call(0xa0, 0x10);
        b.int(0).int(1).call(0xa0, 0x14);
    }
    b.int(32).call(0x80, 0x58).call(0x80, 0x5a);
    if input {
        let start = b.code.len();
        b.int(13).call(0x80, 0x11);
        let target = b.code.len() + 1;
        b.int(0);
        b.code.extend([0x15, 0]);
        b.call(0x80, 0x5f).int(start as i32);
        b.code.push(0x14);
        let end = b.code.len() as i32;
        b.code[target..target + 4].copy_from_slice(&end.to_le_bytes());
    }
    b.ret();
    b.finish()
}
