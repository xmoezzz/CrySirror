// SPDX-License-Identifier: MPL-2.0
//! action315/316 update a packed master and optionally the selected live voice.
use crate::{
    midori_ambient::Ambient,
    midori_music::Music,
    native_numbers::integer2011,
    registers::{Expression, Registers, Value},
    sound::packed_stereo,
};
use anyhow::Result;
use crysirror_core::{
    service::{AudioPcm16GainUpdate, HostServices},
    state::ClientId,
};

#[derive(Debug, PartialEq)]
pub(super) struct Command {
    music: bool,
    result: i32,
    args: [Expression; 4],
}

impl Command {
    pub fn parse(raw: &[u8]) -> Result<Option<Self>> {
        let mut fields = raw.splitn(7, |&c| c == b',');
        let music = match integer2011(fields.next().unwrap_or_default()) {
            315 => true,
            316 => false,
            _ => return Ok(None),
        };
        let result = integer2011(fields.next().unwrap_or_default());
        let mut arg = || Expression::parse(fields.next().unwrap_or_default());
        Ok(Some(Self {
            music,
            result,
            args: [arg()?, arg()?, arg()?, arg()?],
        }))
    }

    pub fn apply(
        self,
        host: &mut dyn HostServices,
        client: ClientId,
        regs: &mut Registers,
        music: &mut Music,
        ambience: &mut Ambient,
    ) -> Result<()> {
        let [a, b, c, d] = self.args;
        let master = a.eval(regs)?.clamp(0, 256);
        let update_live = b.eval(regs)? == 1;
        let level = c.eval(regs)?;
        d.eval(regs)?;
        let slot = if self.music { 31 } else { 32 };
        let previous = regs.read(Value::System(slot))?;
        regs.set_system(slot, master << 16 | master)?;
        if update_live {
            let selected = if self.music {
                music.live_volume()
            } else {
                ambience.live_volume()
            };
            if let Some((channel, scene)) = selected {
                // correctVolume uses signed low-word multiplication and division.
                // The mixer ignores negative corrected values; zero is applied.
                if master.wrapping_mul(level) / 256 >= 0 {
                    let stereo = packed_stereo(regs, slot, [level; 2])?;
                    if let Err(error) = host.update_audio_pcm16_gain(
                        client,
                        channel,
                        AudioPcm16GainUpdate::Stereo(stereo),
                    ) {
                        regs.set_system(slot, previous)?;
                        return Err(error.into());
                    }
                }
                scene.set_levels([level; 2]);
            }
        }
        if self.result >= 0 {
            regs.set_local(self.result, 0)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VmAdapter;
    use crate::{
        Script, midori_ambient, midori_music,
        midori_sound_capture::SoundCapture,
        scenario_load_tests::{scenario, vm},
    };
    use crysirror_core::{
        Error, Result as CoreResult,
        path::VfsPath,
        service::{AudioPreparation, AudioPreparePhase, AudioRequest, FileSystem, InputFrame},
        state::Command as HostCommand,
    };

    #[derive(Default)]
    struct Mixer {
        gains: Vec<(u32, AudioPcm16GainUpdate)>,
    }
    impl FileSystem for Mixer {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> CoreResult<Vec<u8>> {
            Err(Error::NotFound)
        }
        fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Mixer {
        fn submit(&mut self, _: ClientId, _: &[HostCommand]) -> CoreResult<()> {
            Ok(())
        }
        fn now_ms(&self) -> u64 {
            0
        }
        fn begin_audio(&mut self, _: ClientId, _: AudioRequest) -> CoreResult<u64> {
            Ok(1)
        }
        fn audio_preparation(&mut self, _: ClientId, _: u64) -> CoreResult<AudioPreparation> {
            Ok(AudioPreparation {
                phase: AudioPreparePhase::Ready,
                channel: 0,
                error: None,
            })
        }
        fn release_audio(&mut self, _: ClientId, _: u64) -> CoreResult<()> {
            Ok(())
        }
        fn update_audio_pcm16_gain(
            &mut self,
            _: ClientId,
            channel: u32,
            update: AudioPcm16GainUpdate,
        ) -> CoreResult<()> {
            self.gains.push((channel, update));
            Ok(())
        }
    }

    #[test]
    fn midori_music_and_ambient_actions_update_live_gain_and_native_checkpoint() -> Result<()> {
        let mut vm = vm(3)?;
        let mut host = Mixer::default();
        vm.registers.set_system(30, 0)?;
        vm.registers.set_local(20, 19)?;
        let mut records = Vec::new();
        for packet in [
            "315,20,300,1,128", // Master clamp, ignore absent channel.
            "315,-1,128,1,256", // BGM gain table, channel 0.
            "316,-1,-3,1,256",  // Negative master clamps to zero, ambience 4.
            "315,-1,256,2,32",  // Only an exact 1 updates a live channel.
            "315,-1,256,1,-1",  // Negative corrected word keeps previous gain.
            "315,-1,0,1,-1",    // Zero times -1 yields a zero gain update.
        ] {
            records.extend(b"\xbf\0\0\x01");
            records.extend(packet.as_bytes());
            records.push(0);
        }
        vm.script = Some(Script::new(scenario(3, &records, 6), 3)?);
        let step = |vm: &mut crate::Vm, host: &mut Mixer| -> Result<()> {
            vm.step(host, InputFrame::default(), 1)?;
            Ok(())
        };
        step(&mut vm, &mut host)?;
        assert_eq!(vm.registers.read(Value::Local(20))?, 0);
        assert_eq!(vm.registers.read(Value::System(31))?, 0x100_0100);
        assert!(host.gains.is_empty());

        let music = midori_music::Track::parse(b"8,0,144,96,1,0,0,294981,-1")?;
        vm.midori_music
            .play(&mut host, vm.client, &vm.registers, music, false)?;
        vm.midori_music
            .poll(&mut host, vm.client, &mut vm.registers)?;
        vm.midori_ambient.define(midori_ambient::Definition::parse(
            b"2,50,2,0,192,96,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1",
        )?);
        let play = midori_ambient::Command::parse(0x34, 0, &[2, 0, -1, 64])?;
        vm.midori_ambient
            .issue(&mut host, vm.client, &vm.registers, play, false)?;
        vm.midori_ambient.poll(&mut host, vm.client)?;
        step(&mut vm, &mut host)?;
        step(&mut vm, &mut host)?;
        assert_eq!(vm.registers.read(Value::System(31))?, 0x0080_0080);
        assert_eq!(vm.registers.read(Value::System(32))?, 0);
        assert_eq!(
            host.gains,
            [
                (0, AudioPcm16GainUpdate::Stereo([130, 130])),
                (4, AudioPcm16GainUpdate::Stereo([0, 0]))
            ]
        );
        let scene = SoundCapture::from_owners(&vm.midori_music, &vm.midori_ambient).unwrap();
        assert!(scene.text.1.contains(";0,8,2.256.256,65,0"));
        assert!(scene.text.1.contains(";0,2,2.256.256,512,0"));
        step(&mut vm, &mut host)?;
        step(&mut vm, &mut host)?;
        assert_eq!(host.gains.len(), 2);
        let scene = SoundCapture::from_owners(&vm.midori_music, &vm.midori_ambient).unwrap();
        assert!(scene.text.1.contains(";0,8,2.-1.-1,65,0"));
        step(&mut vm, &mut host)?;
        assert_eq!(
            host.gains.last(),
            Some(&(0, AudioPcm16GainUpdate::Stereo([0, 0])))
        );
        Ok(())
    }
}
