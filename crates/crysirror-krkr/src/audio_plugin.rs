//! Native codec module names bind to the real engine decoder registry. The
//! original modules export ITSSModule/ITSSWaveDecoder, not a TJS class. Their
//! byte decoding, scheduling and mixing are shared CrySirror capabilities.
//! Source references: docs/third-party/krkr-audio-codecs.txt.
use crate::natives::*;
use crysirror_tjs2::{Result, TjsError, runtime::Variant};
pub(crate) fn link(rt: &mut Rt, name: &str) -> Result<()> {
    let codec = match name {
        "kropus.dll" => "opus",
        "wuvorbis.dll" => "vorbis",
        _ => return Err(TjsError::runtime("unknown native audio codec module")),
    };
    if !rt.host().audio_codecs.contains(codec) {
        return Err(unsupported(
            "Plugins.link",
            "required shared audio decoder is unavailable",
        ));
    }
    if let Some(arguments) = rt.global_member("__crysirror_arguments").object_handle() {
        let options: &[&str] = if codec == "opus" {
            &["-opus_gain", "-opus_pcm_format"]
        } else {
            &["-vorbis_gain", "-vorbis_rg", "-vorbis_pcm_format"]
        };
        for option in options {
            if !matches!(rt.object_member(arguments, option), Variant::Void) {
                return Err(unsupported("native audio codec option", option));
            }
        }
    }
    Ok(())
}
