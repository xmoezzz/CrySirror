// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native74 parameters map into shared fixed sampling and progress envelopes.
//! All pixel traversal and byte arithmetic live in crysirror-image workers.
use anyhow::{ensure,Context,Result};
use crysirror_image::{effects::ImageEffect,piecewise_resample::{FixedSampleRegion,PiecewiseResampleSpec,ResampleAlpha}};
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct BuraState {
    pub supplied:Vec<i32>,pub frames:i32,pub count:i32,
    pub center_from:[i32;2],pub center_to:[i32;2],
    pub scale_from:i32,pub scale_to:i32,pub scale_curve:i32,pub scale_subframe:i32,
    pub blend_from:i32,pub blend_to:i32,pub blend_curve:i32,pub blend_subframe:i32,
}
impl BuraState {
    pub fn parse(v:&[i32],screen:[u32;2])->Result<Self>{
        ensure!(v.len()<=16&&screen.iter().all(|n|(4..=8192).contains(n)),"BURA native arguments/screen profile");
        let get=|i:usize,d:i32|v.get(i).copied().unwrap_or(d);
        let to=[get(1,screen[0]as i32/2).clamp(1,screen[0]as i32-2),get(2,screen[1]as i32/2).clamp(1,screen[1]as i32-2)];
        let scale_to=get(3,105);let blend_to=get(5,20);
        let s=Self{supplied:v.to_vec(),frames:get(0,20),count:0,center_to:to,center_from:[get(11,to[0]),get(12,to[1])],scale_to,scale_from:get(4,scale_to),scale_curve:get(7,0),scale_subframe:get(8,0),blend_to,blend_from:get(6,blend_to),blend_curve:get(9,0),blend_subframe:get(10,0)};
        ensure!((0..=864000).contains(&s.frames),"BURA native frame bounds");
        s.image_effect(screen)?;Ok(s)
    }
    pub fn image_effect(&self,screen:[u32;2])->Result<ImageEffect>{
        let maximum=i64::from(self.frames.max(1));let count=i64::from(self.count.clamp(0,self.frames.max(1)));
        let mut center=[0i32;2];
        for i in 0..2 {
            let product=(i64::from(self.center_to[i])-i64::from(self.center_from[i]))*count;
            ensure!(i32::try_from(product).is_ok(),"BURA original signed coordinate overflow profile");
            center[i]=i32::try_from(i64::from(self.center_from[i])+product/maximum)?;
            ensure!(center[i]>0&&center[i]<screen[i]as i32,"BURA moving centre outside screen profile");
        }
        let start=self.scale_from.checked_mul(100).context("BURA native scale overflow profile")?;
        let end=self.scale_to.checked_mul(100).context("BURA native scale overflow profile")?;
        let scale=f64::from(crate::native_curve::value(self.count,self.frames,start,end,self.scale_subframe,self.scale_curve)?)/100.0;
        let scale=scale.max(100.0);
        let percent=crate::native_curve::value(self.count,self.frames,self.blend_from,self.blend_to,self.blend_subframe,self.blend_curve)?;
        let sw=percent.checked_mul(256).context("BURA native blend overflow profile")?/100;
        let dw=100i32.checked_sub(percent).and_then(|n|n.checked_mul(256)).context("BURA native complement overflow profile")?/100;
        let mut regions=Vec::with_capacity(4);
        for bottom in [false,true]{for right in [false,true]{
            let reverse=[right,bottom];let mut size=[0u32;2];let mut origin=[0i32;2];let mut step=[1i8;2];let mut src=[0i64;2];let mut delta=[0i64;2];
            for axis in 0..2 {
                let extent=if reverse[axis]{screen[axis]as i32-center[axis]}else{center[axis]};
                ensure!(extent>0,"BURA empty quadrant");size[axis]=extent as u32;
                origin[axis]=if reverse[axis]{screen[axis]as i32-1}else{0};step[axis]=if reverse[axis]{-1}else{1};
                let c=f64::from(center[axis]);let span=f64::from(extent);
                let initial=if reverse[axis]{(span-0.0)*100.0/scale+c}else{c-span*100.0/scale};
                src[axis]=((initial*65536.0)as i64).clamp(0,i64::from(screen[axis])*65536-1);
                delta[axis]=(((c-initial)/span)*65536.0)as i64;
            }
            regions.push(FixedSampleRegion{destination_origin:origin,destination_step:step,size,source_origin_q16:src,source_step_q16:delta});
        }}
        let spec=PiecewiseResampleSpec{regions,source_weight256:sw.clamp(0,256)as u16,destination_weight256:dw.clamp(0,256)as u16,alpha:ResampleAlpha::PreserveDestination};spec.validate()?;
        Ok(ImageEffect::PiecewiseResample{spec})
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn original_quadrants_use_signed_fixed_steps_and_exact_q8_not_regular_resize(){
        let state=BuraState::parse(&[20,2,2,200,200,100,100],[4,4]).unwrap();
        let input=crysirror_image::ImageSurface{width:4,height:4,rgba:(0..16).flat_map(|i|[i,i+20,i+40,255]).collect()};
        let result=input.apply_effect(&state.image_effect([4,4]).unwrap()).unwrap();
        assert_eq!(result.rgba.chunks_exact(4).map(|p|p[0]).collect::<Vec<_>>(),[5,5,6,7,5,5,6,7,9,9,10,11,13,13,14,15]);
        assert_eq!(input.rgba[0],0);
        let state=BuraState::parse(&[20,2,2,100,100,10,10],[4,4]).unwrap();
        let white=crysirror_image::ImageSurface{width:4,height:4,rgba:vec![255;64]};
        assert!(white.apply_effect(&state.image_effect([4,4]).unwrap()).unwrap().rgba.chunks_exact(4).all(|p|p==[254,254,254,255]));
        let mut native=BuraState::parse(&[60,400,300,201,101,0,20,30],[800,600]).unwrap();native.count=15;assert!(native.image_effect([800,600]).is_ok());
        assert!(BuraState::parse(&[20,2,2,100,100,10,10,1],[4,4]).is_err());
    }
}
