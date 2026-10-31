// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original six EFFECT rectangles and API coordinate translation. These are
//! native parameters/save fields, not a pixel or composition implementation.
use anyhow::{ensure,Result};
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct NativeRects {
    pub source:[i32;4],pub destination:[i32;4],
    pub source_from:[i32;4],pub source_to:[i32;4],
    pub destination_from:[i32;4],pub destination_to:[i32;4],
}
impl NativeRects {
    pub fn natural(size:[u32;2])->Self {let r=[0,0,size[0] as i32,size[1] as i32];Self{source:r,destination:r,source_from:r,source_to:r,destination_from:r,destination_to:r}}
    /// Legacy SetEffectRect0x530cd0 updates indices2/3/5 only. The fields called
    /// right/bottom hold widths/heights in the original non-stretch Blt path.
    pub fn set_destination_pair(&mut self,from:[i32;4],to:[i32;4]) {self.destination_from=from;self.destination_to=to;self.destination=from;}
    /// A plain native picture maps to an ordinary engine rectangle and UVs.
    /// Other source clipping/scaling profiles remain explicit rather than
    /// replacing a native blit with a stretched/clamped texture.
    pub fn plain_layer(&self,size:[u32;2])->Result<([f32;4],[f32;4])> {
        let [sx,sy,_,_]=self.source;let [x,y,w,h]=self.destination;
        ensure!(sx>=0&&sy>=0&&w>0&&h>0&&i64::from(sx)+i64::from(w)<=i64::from(size[0])&&i64::from(sy)+i64::from(h)<=i64::from(size[1]),"NNN plain picture source clipping requires shared unscaled-rectangle mapping");
        Ok(([x as f32,y as f32,w as f32,h as f32],[sx as f32/size[0] as f32,sy as f32/size[1] as f32,(i64::from(sx)+i64::from(w))as f32/size[0]as f32,(i64::from(sy)+i64::from(h))as f32/size[1]as f32]))
    }
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn original_destination_rectangles_do_not_reset_source_or_stretch_uvs(){let mut r=NativeRects::natural([800,1200]);let before=r.source;r.set_destination_pair([0,-200,800,1200],[0,0,800,1200]);assert_eq!(r.source,before);assert_eq!(r.source_to,before);assert_eq!(r.destination,[0,-200,800,1200]);assert_eq!(r.destination_to,[0,0,800,1200]);assert_eq!(r.plain_layer([800,1200]).unwrap(),([0.,-200.,800.,1200.],[0.,0.,1.,1.]));r.set_destination_pair([204,118,394,367],[204,118,394,367]);assert_eq!(r.plain_layer([800,1200]).unwrap().1,[0.,0.,394./800.,367./1200.]);assert!(r.plain_layer([2,2]).is_err());}
}
