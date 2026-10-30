//! Native Movie handle/layer policy. Decode, clock and audio belong to CrySirror.
use super::{motion_manager::MotionManager, prim::PrimType};
use alloc::{string::String, vec::Vec};
use anyhow::Result;

pub const MOVIE_GRAPH_ID: u16 = 4063;
pub const MOVIE_GROUP_PRIM_ID: i16 = 4095;
pub const MOVIE_SPRT_PRIM_ID: i16 = 4094;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovieMode {
    ModalWithAudio,
    LayerNoAudio,
}
#[derive(Debug, Clone)]
pub enum MovieCommand {
    Play {
        epoch: u64,
        name: String,
        audio: bool,
    },
    Stop {
        epoch: u64,
    },
}
pub struct MovieRequest<'a> {
    pub name: &'a str,
    pub mode: MovieMode,
    pub screen_w: u32,
    pub screen_h: u32,
}
#[derive(Debug, Default)]
pub struct VideoPlayerManager {
    playing: bool,
    loaded: bool,
    epoch: u64,
    commands: Vec<MovieCommand>,
}
impl VideoPlayerManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }
    // Native Movie does not freeze script contexts (matching the executable).
    pub fn is_modal_active(&self) -> bool {
        false
    }
    pub fn start(&mut self, request: MovieRequest<'_>, motion: &mut MotionManager) -> Result<()> {
        anyhow::ensure!(self.commands.len() < 64, "native Movie command budget");
        anyhow::ensure!(
            (1..=32767).contains(&request.screen_w) && (1..=32767).contains(&request.screen_h),
            "native Movie canvas"
        );
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("native Movie epoch exhausted"))?;
        self.playing = true;
        self.loaded = true;
        self.commands.push(MovieCommand::Play {
            epoch: self.epoch,
            name: request.name.into(),
            audio: request.mode == MovieMode::ModalWithAudio,
        });
        self.ensure_layer(motion, request.screen_w as i16, request.screen_h as i16);
        Ok(())
    }
    pub fn drain_commands(&mut self) -> Vec<MovieCommand> {
        core::mem::take(&mut self.commands)
    }
    pub fn observe(
        &mut self,
        epoch: u64,
        playing: bool,
        frame: Option<(u32, u32, Vec<u8>)>,
        motion: &mut MotionManager,
    ) -> Result<()> {
        if epoch != self.epoch || !self.loaded {
            return Ok(());
        }
        if let Some((width, height, pixels)) = frame {
            motion.load_texture_from_buff(MOVIE_GRAPH_ID, pixels, width, height)?;
            motion.refresh_prims(MOVIE_GRAPH_ID);
        }
        if !playing {
            self.finish(motion);
        }
        Ok(())
    }
    pub fn stop(&mut self, motion: &mut MotionManager) {
        if self.loaded {
            self.commands.push(MovieCommand::Stop { epoch: self.epoch });
        }
        self.finish(motion);
    }
    fn finish(&mut self, motion: &mut MotionManager) {
        self.playing = false;
        self.loaded = false;
        let pm = &mut motion.prim_manager;
        pm.prim_set_draw(MOVIE_SPRT_PRIM_ID as i32, 0);
        pm.prim_set_draw(MOVIE_GROUP_PRIM_ID as i32, 0);
        pm.unlink_prim(MOVIE_SPRT_PRIM_ID);
        pm.unlink_prim(MOVIE_GROUP_PRIM_ID);
    }
    fn ensure_layer(&self, motion: &mut MotionManager, screen_w: i16, screen_h: i16) {
        let pm = &mut motion.prim_manager;
        pm.prim_init_with_type(MOVIE_GROUP_PRIM_ID, PrimType::PrimTypeGroup);
        pm.prim_set_pos(MOVIE_GROUP_PRIM_ID as i32, 0, 0);
        pm.prim_set_z(MOVIE_GROUP_PRIM_ID as i32, 32767);
        pm.prim_set_draw(MOVIE_GROUP_PRIM_ID as i32, 1);
        pm.prim_init_with_type(MOVIE_SPRT_PRIM_ID, PrimType::PrimTypeSprt);
        pm.prim_set_texture_id(MOVIE_SPRT_PRIM_ID as i32, -2);
        pm.prim_set_uv(MOVIE_SPRT_PRIM_ID as i32, 0, 0);
        pm.prim_set_pos(MOVIE_SPRT_PRIM_ID as i32, 0, 0);
        pm.prim_set_size(MOVIE_SPRT_PRIM_ID as i32, screen_w as i32, screen_h as i32);
        pm.prim_set_alpha(MOVIE_SPRT_PRIM_ID as i32, 255);
        pm.prim_set_blend(MOVIE_SPRT_PRIM_ID as i32, 0);
        pm.prim_set_z(MOVIE_SPRT_PRIM_ID as i32, 32767);
        pm.prim_set_draw(MOVIE_SPRT_PRIM_ID as i32, 1);
        pm.set_prim_group_in(MOVIE_GROUP_PRIM_ID as i32, MOVIE_SPRT_PRIM_ID as i32);
        pm.set_prim_group_in(0, MOVIE_GROUP_PRIM_ID as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subsystem::world::GameData;
    #[test]
    fn engine_completion_and_replacement_do_not_accept_stale_movie_frames() {
        let mut game=GameData::default();
        let manager=&mut game.video_manager;
        let motion=&mut game.motion_manager;
        manager.start(MovieRequest {name:"one.wmv",mode:MovieMode::LayerNoAudio,screen_w:64,screen_h:48},motion).unwrap();
        let first=match manager.drain_commands().remove(0) {MovieCommand::Play {epoch,audio,..}=>{assert!(!audio);epoch},_=>panic!()};
        manager.start(MovieRequest {name:"two.wmv",mode:MovieMode::ModalWithAudio,screen_w:64,screen_h:48},motion).unwrap();
        let second=match manager.drain_commands().remove(0) {MovieCommand::Play {epoch,audio,..}=>{assert!(audio);epoch},_=>panic!()};
        assert!(second>first);
        manager.observe(first,false,Some((0,0,Vec::new())),motion).unwrap();
        assert!(manager.is_playing() && manager.is_loaded());
        manager.observe(second,true,Some((2,2,alloc::vec![255;16])),motion).unwrap();
        assert!(manager.is_playing());
        manager.observe(second,false,None,motion).unwrap();
        assert!(!manager.is_playing() && !manager.is_loaded());
        assert!(manager.drain_commands().is_empty());
    }
}
