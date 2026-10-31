// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Reviewed legacy CTRANS native parameters/count only. Public Image Composite
//! implements clipping, fixed-point blending and immutable output resources.
use anyhow::{ensure,Result};
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct CharaTransState {
    pub frames:i32,pub end_percent:i32,pub start_percent:i32,
    pub easing:i32,pub subframe:i32,pub count:i32,
    pub clip:bool,pub clip_to:[i32;4],pub clip_from:[i32;4],
}
impl CharaTransState {
    pub fn parse(v:&[i32],screen:[u32;2])->Result<Self> {
        // Actual module SetParam0x4b33c0 accepts fourteen fields, not the
        // twenty-three fields in the later public branch.
        ensure!(v.len()<=14,"CTRANS legacy argument count");
        let get=|i:usize,d:i32|v.get(i).copied().unwrap_or(d);
        let clip_to=[get(6,0),get(7,0),get(8,screen[0]as i32),get(9,screen[1]as i32)];
        let clip_from=[get(10,clip_to[0]),get(11,clip_to[1]),get(12,clip_to[2]),get(13,clip_to[3])];
        let state=Self{frames:get(0,20),end_percent:get(1,50),start_percent:get(2,100),easing:get(3,0),subframe:get(4,0),count:0,clip:get(5,0)!=0,clip_to,clip_from};
        ensure!((0..=864000).contains(&state.frames),"CTRANS native frame bounds");
        ensure!(state.easing==0,"CTRANS native nonlinear curve requires shared curve mapping");
        Ok(state)
    }
    fn interpolate(&self,start:i32,end:i32)->i32 {
        let max=self.frames.max(1);
        (f64::from(start)+(f64::from(end)-f64::from(start))*f64::from(self.count.clamp(0,max))/f64::from(max)+0.5)as i32
    }
    pub fn weight256(&self)->u16 {(self.interpolate(self.start_percent,self.end_percent).clamp(0,100)*256/100)as u16}
    pub fn clip_rect(&self)->Option<[i32;4]> {
        self.clip.then(||std::array::from_fn(|i|self.interpolate(self.clip_from[i],self.clip_to[i])))
    }
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn original_ctrans_defaults_rounding_clip_and_persistent_endpoint(){
        let mut s=CharaTransState::parse(&[45,0,100],[800,600]).unwrap();
        assert_eq!(s.weight256(),256);s.count=1;assert_eq!(s.weight256(),250);
        s.count=45;assert_eq!(s.weight256(),0);s.count=99;assert_eq!(s.weight256(),0);
        let d=CharaTransState::parse(&[],[800,600]).unwrap();assert_eq!(d.end_percent,50);assert_eq!(d.clip_to,[0,0,800,600]);
        let mut c=CharaTransState::parse(&[4,50,100,0,0,1,2,4,8,10,0,2,4,6],[800,600]).unwrap();c.count=2;assert_eq!(c.clip_rect(),Some([1,3,6,8]));
        assert!(CharaTransState::parse(&[1,50,100,1],[800,600]).is_err());
        assert!(CharaTransState::parse(&[0;15],[800,600]).is_err());
    }
}
