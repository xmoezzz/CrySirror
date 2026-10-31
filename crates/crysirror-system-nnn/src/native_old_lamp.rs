// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Legacy109 native trigger/count/global-rand policy. Shared image composition
//! owns all drawing; this type holds no pixels, decoder, surface or host.
use anyhow::{ensure,Result};
use crysirror_image::temporal_pattern::PatternRng;
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct OldLampState {
    pub supplied:Vec<i32>,
    pub durations:[i32;3],
    pub flicker:[i32;2],
    pub random_flip:i32,
    pub count:i32,
    pub mode:i32,
    pub remaining:i32,
}
impl OldLampState {
    pub fn parse(v:&[i32])->Result<Self>{
        ensure!(v.len()<=16,"OLDLAMP legacy argument count");
        let get=|i:usize,d:i32|v.get(i).copied().unwrap_or(d);
        let s=Self{supplied:v.to_vec(),durations:[get(1,20),get(2,0),get(3,40)],flicker:[get(4,1),get(5,1)],random_flip:get(6,0),count:0,mode:0,remaining:0};
        ensure!(s.durations.iter().chain(s.flicker.iter()).all(|n|(-1000000..=1000000).contains(n)),"OLDLAMP native finite timing profile");
        Ok(s)
    }
    /// CAllEffect::CalcuOnly runs every active lamp before any layer Print.
    /// The caller carries this candidate together with the shared global RNG;
    /// neither becomes committed native state until engine work succeeds.
    pub fn calcu(&mut self,rng:&mut PatternRng)->Result<()> {
        ensure!((0..=2).contains(&self.mode),"OLDLAMP state mode");
        if self.count>10000{self.count=0;}
        self.remaining-=1;
        if self.remaining>=0{return Ok(())}
        let mut next=0;let mut time=self.durations[0];
        if self.mode==0 {
            let divisor=(self.durations[1]+self.durations[2]).max(1) as u32;
            if rng.next_draw()?%divisor < self.durations[1].max(0)as u32 {next=1;time=self.durations[1];}
            else{next=2;time=self.durations[2];}
        }
        self.mode=next;
        if time>0 {time+=i32::try_from(rng.next_draw()?%(time as u32))?-time/2;}
        self.remaining=time.max(1);Ok(())
    }
    /// Print's optional random inversion consumes the same global stream only
    /// when a picture exists and mode0 is actually printed.
    pub fn visible(&self,has_picture:bool,rng:&mut PatternRng)->Result<bool>{
        if !has_picture||self.mode==2{return Ok(false)}
        if self.mode==1{return Ok(true)}
        ensure!(self.mode==0,"OLDLAMP state mode");
        let divisor=(self.flicker[0]+self.flicker[1]).max(1);
        let mut on=self.remaining%divisor<self.flicker[0];
        if self.random_flip>0 && i32::try_from(rng.next_draw()?%100)?<self.random_flip {on=!on;}
        Ok(on)
    }
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn original_lamp_calcu_and_print_keep_one_global_stream_with_raw_frame_argument(){
        let mut s=OldLampState::parse(&[40,40,0,10,11,1,0]).unwrap();let mut rng=PatternRng::msvc(1);
        assert_eq!(s.supplied[0],40);assert!(s.visible(true,&mut rng).unwrap());assert_eq!(rng.draws,0);
        s.calcu(&mut rng).unwrap();assert_eq!(s.mode,2);assert_eq!(s.remaining,12);assert_eq!(rng.draws,2);assert!(!s.visible(true,&mut rng).unwrap());
        for _ in 0..12{s.calcu(&mut rng).unwrap();}assert_eq!(s.remaining,0);assert_eq!(rng.draws,2);
        s.calcu(&mut rng).unwrap();assert_eq!(s.mode,0);assert_eq!(rng.draws,3);
        let before=rng.clone();assert!(!s.visible(false,&mut rng).unwrap());assert_eq!(rng,before);
        s.count=10001;s.calcu(&mut rng).unwrap();assert_eq!(s.count,0);
        assert!(OldLampState::parse(&[0;17]).is_err());
    }
}
