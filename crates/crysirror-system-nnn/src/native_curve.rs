// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native0x482070 bit-field decoding and final integer ABI quantization. The
//! reusable envelope/shape arithmetic is the engine's public ProgressCurve.
use anyhow::{ensure,Result};
use crysirror_image::progress_curve::{ProgressCurve,ProgressKnot,ProgressShape};
pub fn curve(max:i32,subframe:i32,kind:i32)->Result<ProgressCurve>{
    ensure!((0..200).contains(&kind),"NNN native random curve requires the shared global RNG transaction");
    let code=kind%100;let warp=code%15;
    let shape=match code/15 {2=>ProgressShape::QuadraticIn,4=>ProgressShape::SquareRoot,5=>ProgressShape::HalfCosine{pi:3.14159},_=>ProgressShape::Linear};
    let max=max.max(1)as u64;let edge=subframe.max(0)as u64;
    let knots=match warp {
        0=>vec![ProgressKnot{at:0,value:0.0},ProgressKnot{at:max,value:1.0}],
        4 if edge==0=>vec![ProgressKnot{at:0,value:1.0},ProgressKnot{at:max,value:1.0},ProgressKnot{at:max,value:0.0}],
        4 if edge>max=>vec![ProgressKnot{at:0,value:0.0},ProgressKnot{at:edge,value:1.0}],
        4 if edge*2<=max=>vec![ProgressKnot{at:0,value:0.0},ProgressKnot{at:edge,value:1.0},ProgressKnot{at:max-edge,value:1.0},ProgressKnot{at:max,value:0.0}],
        4=>vec![ProgressKnot{at:0,value:0.0},ProgressKnot{at:edge,value:1.0},ProgressKnot{at:edge,value:(max-edge)as f64/edge as f64},ProgressKnot{at:max,value:0.0}],
        _=>anyhow::bail!("NNN native curve time-map {warp} is not mapped to a shared envelope"),
    };
    let curve=ProgressCurve{knots,shape};curve.validate()?;Ok(curve)
}
pub fn value(count:i32,max:i32,start:i32,end:i32,subframe:i32,kind:i32)->Result<i32>{
    let progress=curve(max,subframe,kind)?.sample(count.clamp(0,max.max(1))as u64)?;
    let (start,end)=if kind/100%2!=0{(end,start)}else{(start,end)};
    Ok((f64::from(start)+(f64::from(end)-f64::from(start))*progress+0.5)as i32)
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn original_linear_quadratic_and_zero_edge_pulse_map_without_fake_profiles(){
        assert_eq!(value(15,60,10100,20100,0,30).unwrap(),10725);
        assert_eq!(value(15,60,100,0,0,0).unwrap(),75);
        assert_eq!(value(0,50,256,128,0,4).unwrap(),128);
        assert_eq!(value(49,50,256,128,0,4).unwrap(),128);
        assert_eq!(value(50,50,256,128,0,4).unwrap(),256);
        assert_eq!(value(0,50,10,20,0,100).unwrap(),20);
        assert!(curve(20,0,1).is_err());assert!(curve(20,0,200).is_err());
    }
}
