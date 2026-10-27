// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! CPAL is confined to the device worker and its callback capabilities.
use crate::pcm_output::{
    OUTPUT_RATE, OpenedOutput, OutputBackend, OutputConsumer, OutputFormat, OutputSignals,
    OutputStream,
};
use anyhow::{Context, Result, bail, ensure};
use cpal::{
    SampleFormat, SampleRate, StreamConfig, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::time::Duration;
pub(crate) struct CpalBackend;
struct Stream {
    _stream: cpal::Stream,
}
impl OutputStream for Stream {}
fn config(device: &cpal::Device) -> Result<SupportedStreamConfig> {
    let default = device
        .default_output_config()
        .context("default output configuration")?;
    // Prefer native stereo 48k; retain the device default when that is unavailable.
    if let Ok(configs) = device.supported_output_configs() {
        let mut choices = configs
            .filter(|r| {
                r.channels() == 2
                    && r.min_sample_rate().0 <= OUTPUT_RATE
                    && r.max_sample_rate().0 >= OUTPUT_RATE
            })
            .collect::<Vec<_>>();
        choices.sort_by_key(|r| match r.sample_format() {
            SampleFormat::F32 => 0,
            SampleFormat::I16 => 1,
            SampleFormat::F64 => 2,
            _ => 3,
        });
        if let Some(choice) = choices.into_iter().next() {
            return Ok(choice.with_sample_rate(SampleRate(OUTPUT_RATE)));
        }
    }
    Ok(default)
}
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &StreamConfig,
    consumer: OutputConsumer,
    signals: OutputSignals,
) -> Result<cpal::Stream> {
    let mut consumer = consumer.configure(config.sample_rate.0, config.channels)?;
    let callback_signals = signals.clone();
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                if let Err(error) = consumer.fill(out, T::from_sample) {
                    callback_signals.fail(error);
                }
            },
            move |error| signals.fail(error),
            Some(Duration::from_secs(3)),
        )
        .context("build output stream")
}
impl OutputBackend for CpalBackend {
    fn open(&self, consumer: OutputConsumer, signals: OutputSignals) -> Result<OpenedOutput> {
        ensure!(!signals.is_cancelled(), "audio opening cancelled");
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no mixer audio device; use --no-audio for intentionally silent operation")?;
        let supported = config(&device)?;
        let sample_format = supported.sample_format();
        let config = supported.config();
        let info = OutputFormat {
            sample_rate: config.sample_rate.0,
            channels: config.channels,
            sample_format: sample_format.to_string(),
        };
        ensure!(!signals.is_cancelled(), "audio opening cancelled");
        let cancellation = signals.clone();
        let stream = match sample_format {
            SampleFormat::F32 => build::<f32>(&device, &config, consumer, signals),
            SampleFormat::F64 => build::<f64>(&device, &config, consumer, signals),
            SampleFormat::I8 => build::<i8>(&device, &config, consumer, signals),
            SampleFormat::I16 => build::<i16>(&device, &config, consumer, signals),
            SampleFormat::I32 => build::<i32>(&device, &config, consumer, signals),
            SampleFormat::I64 => build::<i64>(&device, &config, consumer, signals),
            SampleFormat::U8 => build::<u8>(&device, &config, consumer, signals),
            SampleFormat::U16 => build::<u16>(&device, &config, consumer, signals),
            SampleFormat::U32 => build::<u32>(&device, &config, consumer, signals),
            SampleFormat::U64 => build::<u64>(&device, &config, consumer, signals),
            _ => bail!("unsupported output sample format: {sample_format}"),
        }?;
        ensure!(!cancellation.is_cancelled(), "audio opening cancelled");
        stream.play().context("start output stream")?;
        OpenedOutput::new(info, Stream { _stream: stream })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_integer_conversion_retains_silence_and_full_scale_without_io() {
        assert_eq!(<i16 as cpal::Sample>::from_sample(0f32), 0);
        assert_eq!(<i16 as cpal::Sample>::from_sample(-1f32), i16::MIN);
        assert_eq!(<i16 as cpal::Sample>::from_sample(1f32), i16::MAX);
        assert_eq!(<u16 as cpal::Sample>::from_sample(0f32), 32768);
        assert_eq!(<u16 as cpal::Sample>::from_sample(-1f32), u16::MIN);
        assert_eq!(<u16 as cpal::Sample>::from_sample(1f32), u16::MAX);
        assert_eq!(<f64 as cpal::Sample>::from_sample(0.25f32), 0.25);
    }
}
