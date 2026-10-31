// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original outer modes. Configuration, counters and transitions remain native;
//! resource reads, image transforms/composition, media and input are services.
use crate::{config::Names,native_bootstrap::NativeConfiguration,native_resources::ImageLookup};
use anyhow::{ensure,Context,Result};
use crysirror_core::{path::{Mount,VfsPath},service::ClientStorage};
use crysirror_tasks::Cancellation;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum NativeMode {Nothing=1,Logo=2,Opening=3,Title=4,Movie=27}
impl NativeMode {
    pub fn named(name:&[u8])->Result<Self> {
        for (n,m) in [(b"nothing".as_slice(),Self::Nothing),(b"logo",Self::Logo),(b"opening",Self::Opening),(b"title",Self::Title),(b"printmovie",Self::Movie)] {if n.eq_ignore_ascii_case(name){return Ok(m)}}
        anyhow::bail!("NNN unreviewed outer native mode {}",String::from_utf8_lossy(name))
    }
}
#[derive(Clone,Debug)]
pub struct NativeLogoSpec {
    pub source:ImageLookup,
    pub frames:[u32;3],
    pub colors:[[u8;3];2],
    pub skip_after_launch:u32,
}
#[derive(Clone,Debug)]
pub struct NativeStartupProfile {
    pub nothing_color:[u8;3],
    pub nothing_next:NativeMode,
    pub codec_check:bool,
    pub codec_limit:i32,
    pub startup_jump_script:bool,
    pub logos:Vec<NativeLogoSpec>,
    pub logo_next:NativeMode,
    pub opening_next:NativeMode,
    pub opening_script:Vec<u8>,
    pub opening_loop:bool,
    pub title_script:Vec<u8>,
    pub title_loop:bool,
    pub title:Names,
    pub movie:Names,
    pub movie_asset:String,
    pub codec_asset:String,
}
impl NativeStartupProfile {
    /// Invoke on a scoped bootstrap worker. No optional raw OS file fallback.
    pub fn read(config:&NativeConfiguration,storage:&dyn ClientStorage,cancel:&Cancellation)->Result<Self> {
        let read=|name:&str|->Result<Names>{
            cancel.check()?;
            for (folder,ext,cipher) in [("init2","fxf",true),("init","xtx",false)] {
                let path=config.resources.path(&format!("{folder}/{name}.{ext}"))?;
                let reader=match storage.open_asset(&VfsPath::new(Mount::Asset,&path)?) {Ok(r)=>r,Err(crysirror_core::Error::NotFound)=>continue,Err(e)=>return Err(e.into())};
                let bytes=crysirror_resource::read_all(reader.as_ref(),1<<20)?;return Names::parse(&bytes,cipher);
            }
            anyhow::bail!("NNN original {name} startup configuration absent")
        };
        Self::from_tables(config,&read("nothing")?,&read("logo")?,&read("opening")?,read("title")?,read("printmovie")?)
    }
    pub fn from_tables(config:&NativeConfiguration,nothing:&Names,logo:&Names,opening:&Names,title:Names,movie:Names)->Result<Self> {
        let int=|n:&Names,key:&str,default:i32|->Result<i32>{Ok(n.integer(key.as_bytes())?.unwrap_or(default))};
        let mode=|n:&Names,key:&str,default:NativeMode|->Result<NativeMode>{n.parameter(key.as_bytes()).map(NativeMode::named).transpose().map(|m|m.unwrap_or(default))};
        let color=|n:&Names,prefix:&str,default:[u8;3]|->Result<[u8;3]>{let mut c=default;for (i,ch) in ["R","G","B"].iter().enumerate(){c[i]=int(n,&format!("{prefix}{ch}"),i32::from(c[i]))? as u8;}Ok(c)};
        let count=int(logo,"logoNumber",2)?;ensure!((1..=64).contains(&count),"NNN original logo count bounds");
        ensure!(int(logo,"soundNumber",0)?==0,"NNN timed logo system sounds require native trigger policy");
        let use_files=int(logo,"useCustomFileFlag",0)?!=0;let use_times=int(logo,"useCustomTimeFlag",0)?!=0;
        let use_colors=int(logo,"useCustomColorFlag",0)?!=0;let use_skip=int(logo,"useCustomSkipFlag",0)?!=0;
        let use_custom_bg=int(logo,"useCustomBGColorFlag",0)?!=0;
        let default_bg=int(logo,"defaultUseBGColor",0)?;
        let default_times=[int(logo,"defaultFadeInTime",10)?,int(logo,"defaultKeepTime",20)?,int(logo,"defaultFadeOutTime",10)?];
        let default_colors=[color(logo,"defaultFadeInColor",[255;3])?,color(logo,"defaultFadeOutColor",[255;3])?];
        let levels=u32::try_from(int(&config.setup,"dwqPackLevel",0)?)?;
        let mut logos=Vec::new();
        for i in 1..=count {
            let mut times=default_times;
            if use_times {for (n,key) in ["fadeInTime","keepTime","fadeOutTime"].iter().enumerate(){times[n]=int(logo,&format!("{key}{i}"),times[n])?;}}
            ensure!(times.iter().all(|n|(0..=864000).contains(n)),"NNN logo frame limits");
            let mut colors=default_colors;
            if use_colors {for (n,prefix) in ["fadeInColor","fadeOutColor"].iter().enumerate(){let mut c=colors[n];for (ch,letter) in ["R","G","B"].iter().enumerate(){c[ch]=int(logo,&format!("{prefix}{letter}{i}"),i32::from(c[ch]))? as u8;}colors[n]=c;}}
            for phase in ["fadeInBGColorFlag","fadeOutBGColorFlag"] {let bg=if use_custom_bg{int(logo,&format!("{phase}{i}"),default_bg)?}else{default_bg};ensure!(bg==0,"NNN logo first-texel background colour requires shared pixel-sample capability");}
            let path=if use_files {
                let file=logo.parameter(format!("filename{i}").as_bytes()).context("NNN custom logo filename absent")?;
                if file.get(2)==Some(&b'\\')||file.get(3)==Some(&b'\\'){file.to_vec()}else{[b"sys\\".as_slice(),file].concat()}
            } else {format!("sys\\logo{i}").into_bytes()};
            let skip=if use_skip{int(logo,&format!("skipOK{i}"),1)?}else{1};
            logos.push(NativeLogoSpec{source:config.resources.picture(&path,levels)?,frames:times.map(|n|n as u32),colors,skip_after_launch:u32::try_from(skip)?});
        }
        let back=|n:&Names,default_loop:i32|->Result<(Vec<u8>,bool)>{
            ensure!(int(n,"backScriptFlag",0)?!=0,"NNN startup mode requires its original background script profile");
            let name=n.parameter(b"backScriptName").context("NNN original background script absent")?;ensure!(config.project.index(name).is_some(),"NNN background script absent from original project table");
            Ok((name.to_vec(),int(n,"backScriptLoopFlag",default_loop)?!=0))
        };
        let (opening_script,opening_loop)=back(opening,0)?;let (title_script,title_loop)=back(&title,1)?;
        let movie_name=movie.parameter(b"movieFileName").unwrap_or(b"default.wmv");
        let codec_name=movie.parameter(b"codecCheckFileName").unwrap_or(movie_name);
        let movie_asset=config.resources.encoded_path(&[b"movie\\".as_slice(),movie_name].concat())?;
        let codec_asset=config.resources.encoded_path(&[b"movie\\".as_slice(),codec_name].concat())?;
        Ok(Self{movie_asset,codec_asset,nothing_color:color(nothing,"fillColor",[255;3])?,nothing_next:mode(nothing,"nextMode",NativeMode::Logo)?,codec_check:int(nothing,"codecCheckFlag",0)?!=0,codec_limit:int(nothing,"codecLimit",3)?,startup_jump_script:int(nothing,"startupJumpScriptFlag",0)?!=0,logos,logo_next:mode(logo,"nextMode",NativeMode::Title)?,opening_next:mode(opening,"nextMode",NativeMode::Title)?,opening_script,opening_loop,title_script,title_loop,title,movie})
    }
}

use crysirror_core::{image_processing::{ImageInput,ImageJob,ImageProcessState,ImageResource,ImageCompositeLayer,ImageCompositeMath},service::{HostServices,InputFrame},state::ClientId};
use crysirror_tasks::{Task,TaskPool};
use std::sync::Arc;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
struct LogoFrame {index:usize,phase:usize,percent:u16}
/// A native outer Logo mode with immutable images, public worker operations
/// and ordinary Layers. No pixels, decoder, window or device live in this type.
pub struct NativeLogoPlayer {
    client:ClientId,
    layer:u32,
    canvas:[u32;2],
    frame_ms:u64,
    launch_count:u32,
    specs:Vec<NativeLogoSpec>,
    resources:crate::resource_bridge::NativeResources,
    pool:TaskPool,
    source_task:Option<Task<crate::resource_bridge::NativeImageSource>>,
    source_job:Option<ImageJob>,
    source:Option<Arc<dyn ImageResource>>,
    frame_job:Option<(LogoFrame,ImageJob)>,
    presented:Option<LogoFrame>,
    published:Option<String>,
    index:usize,
    phase:usize,
    count:u32,
    clock:Option<u64>,
    remainder:u64,
    first_ready:bool,
    finished:bool,
    pending_buttons:u64,
    pub api_calls:u64,
}
impl NativeLogoPlayer {
    pub fn new(client:ClientId,layer:u32,canvas:[u32;2],frame_ms:u64,launch_count:u32,specs:Vec<NativeLogoSpec>,resources:crate::resource_bridge::NativeResources)->Result<Self> {
        ensure!(client.0!=0&&layer!=0&&canvas.iter().all(|n|(1..=8192).contains(n))&&(1..=10000).contains(&frame_ms)&&!specs.is_empty()&&specs.len()<=64,"NNN native logo scope/bounds");
        Ok(Self{client,layer,canvas,frame_ms,launch_count,specs,resources,pool:TaskPool::new("nnn-native-logo-source",1,1)?,source_task:None,source_job:None,source:None,frame_job:None,presented:None,published:None,index:0,phase:0,count:0,clock:None,remainder:0,first_ready:false,finished:false,pending_buttons:0,api_calls:0})
    }
    pub fn preparation_pending(&self)->bool {!self.first_ready&&!self.finished}
    pub fn dependency_pending(&self)->bool {self.source_task.is_some()||self.source_job.is_some()||self.frame_job.is_some()}
    pub fn native_location(&self)->(usize,usize,u32){(self.index,self.phase,self.count)}
    fn next(&mut self) {self.index+=1;self.phase=0;self.count=0;self.source=None;self.presented=None;self.clock=None;self.remainder=0;}
    fn elapse(&mut self,now:u64,input:InputFrame)->Result<()> {
        if self.source.is_none(){return Ok(())}
        let spec=&self.specs[self.index];
        if spec.skip_after_launch!=0&&spec.skip_after_launch<=self.launch_count.saturating_add(1)&&input.buttons_pressed&3!=0 {self.next();return Ok(())}
        let previous=self.clock.replace(now).unwrap_or(now);
        ensure!(now>=previous,"NNN logo clock moved backwards");
        self.remainder=self.remainder.checked_add(now-previous).context("NNN logo clock overflow")?;
        // Keep remaining debt, rather than unboundedly iterating a long gap on
        // the owner. There is no I/O wait or sleep in this native clock step.
        let steps=(self.remainder/self.frame_ms).min(4096);self.remainder-=steps*self.frame_ms;
        for _ in 0..steps {
            self.count+=1;
            if self.count>=self.specs[self.index].frames[self.phase] {
                self.count=0;self.phase+=1;
                if self.phase==3 {self.next();break}
            }
        }
        Ok(())
    }
    fn target(&self)->LogoFrame {
        let spec=&self.specs[self.index];
        let percent=match self.phase {
            0=>u64::from(self.count)*100/u64::from(spec.frames[0].max(1)),
            1=>100,
            _=>u64::from(spec.frames[2].max(1).saturating_sub(self.count))*100/u64::from(spec.frames[2].max(1)),
        }.min(100) as u16;
        LogoFrame{index:self.index,phase:self.phase,percent}
    }
    /// Returns true only after the actual native page sequence ends. A completed
    /// read/process receipt is owner-published before its predecessor is released.
    pub fn poll(&mut self,host:&mut dyn HostServices,input:InputFrame)->Result<bool> {
        if self.finished{return Ok(true)}
        self.pending_buttons|=input.buttons_pressed&3;
        if let Some((key,job))=&mut self.frame_job {
            match job.poll()? {
                ImageProcessState::Pending=>return Ok(false),
                ImageProcessState::Failed(error)=>anyhow::bail!("NNN logo common composition: {error}"),
                ImageProcessState::Ready(image)=>{
                    ensure!(image.client()==self.client&&image.size()==self.canvas,"NNN logo frame resource scope");
                    let published=host.publish_image_resource(self.client,self.layer,image)?;self.api_calls+=1;
                    let path=published.path.relative().to_owned();let [w,h]=self.canvas;
                    let batch=serde_json::to_vec(&serde_json::json!({"layers":[{"op":"put","layer":{"id":self.layer,"rect":[0,0,w,h],"order":[2000,0],"paint":{"kind":"image","asset":path}}}]}))?;
                    if let Err(e)=host.submit_engine(self.client,1,&batch){let _=host.release_image_resource(self.client,&path);return Err(e.into())}
                    self.api_calls+=1;
                    if let Some(old)=self.published.replace(path){host.release_image_resource(self.client,&old)?;}
                    self.presented=Some(*key);self.frame_job=None;self.first_ready=true;
                }
            }
        }
        if let Some(task)=&mut self.source_task {
            let Some(source)=task.try_take()? else{return Ok(false)};
            self.source_job=Some(host.image_metadata(self.client)?.begin_process(source.input,serde_json::to_vec(&[crysirror_image::effects::ImageEffect::OverrideAlpha{alpha:255}])?.into())?);
            self.api_calls+=1;self.source_task=None;
        }
        if let Some(job)=&mut self.source_job {
            match job.poll()? {
                ImageProcessState::Pending=>return Ok(false),
                ImageProcessState::Failed(error)=>anyhow::bail!("NNN logo shared image decode: {error}"),
                ImageProcessState::Ready(image)=>{ensure!(image.client()==self.client,"NNN logo image client");self.source=Some(image);self.source_job=None;self.clock=Some(host.now_ms());}
            }
        }
        if self.source.is_some(){
            self.elapse(host.now_ms(),InputFrame{buttons_pressed:self.pending_buttons,..input})?;
            self.pending_buttons=0;
        }
        if self.index==self.specs.len() {
            if self.published.is_some(){
                host.submit_engine(self.client,1,&serde_json::to_vec(&serde_json::json!({"layers":[{"op":"remove","id":self.layer}]}))?)?;self.api_calls+=1;
            }
            if let Some(path)=self.published.take(){host.release_image_resource(self.client,&path)?;}
            self.finished=true;return Ok(true)
        }
        let Some(source)=&self.source else {
            if self.source_task.is_none(){let spec=self.specs[self.index].source.clone();let resources=self.resources.clone();self.source_task=Some(self.pool.spawn(move|cancel|resources.image_source(&spec,&cancel))?);}
            return Ok(false)
        };
        let key=self.target();if self.presented==Some(key){return Ok(false)}
        let spec=&self.specs[self.index];let color=spec.colors[if self.phase==2{1}else{0}];let size=source.size();
        let input=ImageInput::Composite{base:Box::new(ImageInput::Solid{size:self.canvas,color:[color[0],color[1],color[2],255]}),layers:vec![ImageCompositeLayer{input:Box::new(ImageInput::Resource(source.clone())),source:[0,0,size[0].min(self.canvas[0]) as i32,size[1].min(self.canvas[1]) as i32],destination:[0,0],clip:None,opacity:255,math:ImageCompositeMath::MaskPiecewise256{weight:key.percent*256/100}}]};
        self.frame_job=Some((key,host.image_metadata(self.client)?.begin_process(input,b"[]".as_slice().into())?));self.api_calls+=1;
        Ok(false)
    }
}

/// Reviewed portable host policy: only the actual shared decoder/header
/// capability is known here. This never manufactures the original DirectShow
/// numeric codec status and does not claim that every audio packet can decode.
pub struct NativeStartupShell {
    client:ClientId,
    root:u32,
    pub profile:NativeStartupProfile,
    mode:NativeMode,
    probe:Option<crysirror_core::service::MediaProbeJob>,
    probe_info:Option<crysirror_core::service::MediaProbeInfo>,
    logo:NativeLogoPlayer,
    nothing_published:bool,
    pub api_calls:u64,
}
impl NativeStartupShell {
    pub fn new(client:ClientId,root:u32,canvas:[u32;2],frame_ms:u64,launch_count:u32,profile:NativeStartupProfile,resources:crate::resource_bridge::NativeResources)->Result<Self> {
        ensure!(!profile.startup_jump_script,"NNN native configured startupJumpScript requires InitNewGame transition policy");
        ensure!(profile.nothing_next==NativeMode::Logo&&profile.logo_next==NativeMode::Opening,"NNN unreviewed startup mode chain");
        let logo=NativeLogoPlayer::new(client,9000,canvas,frame_ms,launch_count,profile.logos.clone(),resources)?;
        Ok(Self{client,root,profile,mode:NativeMode::Nothing,probe:None,probe_info:None,logo,nothing_published:false,api_calls:0})
    }
    pub fn preparation_pending(&self)->bool {self.mode==NativeMode::Nothing||(self.mode==NativeMode::Logo&&self.logo.preparation_pending())}
    pub fn dependency_pending(&self)->bool {self.probe.is_some()||self.logo.dependency_pending()}
    pub fn mode(&self)->NativeMode {self.mode}
    pub fn diagnostics(&self)->serde_json::Value {serde_json::json!({"mode":format!("{:?}",self.mode),"logo":self.logo.native_location(),"dependency_pending":self.dependency_pending(),"legacy_codec_limit":self.profile.codec_limit,"portable_codec_policy":"shared_video_header_then_live_media_transport","video_header_verified":self.probe_info.as_ref().map(|i|i.header_validated),"video_decoder":self.probe_info.as_ref().map(|i|i.video_decoder.as_str()),"audio_decoder":self.probe_info.as_ref().and_then(|i|i.audio_decoder.as_deref()),"audio_codec_verified":false,"engine_api_calls":self.api_calls+self.logo.api_calls})}
    /// True hands control to the original configured background SPT, not sys.
    pub fn poll(&mut self,host:&mut dyn HostServices,input:InputFrame)->Result<bool> {
        if self.mode==NativeMode::Nothing {
            if !self.nothing_published {
                let c=self.profile.nothing_color;
                host.submit_engine(self.client,1,&serde_json::to_vec(&serde_json::json!({"layers":[{"op":"put","layer":{"id":self.root,"rect":[0,0,self.logo.canvas[0],self.logo.canvas[1]],"paint":{"kind":"solid","color":[c[0],c[1],c[2],255]}}}]}))?)?;self.api_calls+=1;self.nothing_published=true;
            }
            if self.profile.codec_check&&self.probe_info.is_none() {
                if self.probe.is_none() {self.probe=Some(host.begin_media_probe(self.client,&VfsPath::new(Mount::Asset,&self.profile.codec_asset)?)?);self.api_calls+=1;}
                match self.probe.as_mut().unwrap().poll()? {
                    crysirror_core::service::MediaProbeState::Pending=>return Ok(false),
                    crysirror_core::service::MediaProbeState::Failed(error)=>anyhow::bail!("NNN portable shared video decoder/header check failed (legacy DirectShow status not substituted): {error}"),
                    crysirror_core::service::MediaProbeState::Ready(info)=>{ensure!(info.client==self.client&&info.header_validated&&!info.video_decoder.is_empty(),"NNN shared video header capability not established");self.probe_info=Some(info);self.probe=None;}
                }
            }
            self.mode=NativeMode::Logo;
        }
        if self.mode==NativeMode::Logo {
            if !self.logo.poll(host,input)? {return Ok(false)}
            self.mode=self.profile.logo_next;
        }
        Ok(self.mode==NativeMode::Opening)
    }
}
