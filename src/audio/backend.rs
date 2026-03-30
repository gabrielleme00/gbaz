use super::buffer::{create_sample_queue, SampleConsumer, SampleProducer};
use super::control::{AudioControls, SharedAudioControls};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;

pub struct AudioBackend {
    _stream: cpal::Stream,
    producer: SampleProducer,
    controls: SharedAudioControls,
    sample_rate: u32,
    buffer_capacity: usize,
}

impl AudioBackend {
    pub fn new(target_latency_ms: u32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No default audio output device".to_string())?;

        let supported = device
            .default_output_config()
            .map_err(|e| format!("Failed to query audio config: {e}"))?;

        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let sample_rate = config.sample_rate.0;

        let frames_per_latency =
            ((sample_rate as usize * target_latency_ms.max(10) as usize) / 1000).max(1024);
        let buffer_capacity = frames_per_latency * 4;

        let (producer, consumer) = create_sample_queue(buffer_capacity);
        let controls: SharedAudioControls = Arc::new(AudioControls::new());

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                Self::build_stream::<f32>(&device, &config, consumer, Arc::clone(&controls))?
            }
            cpal::SampleFormat::I16 => {
                Self::build_stream::<i16>(&device, &config, consumer, Arc::clone(&controls))?
            }
            cpal::SampleFormat::U16 => {
                Self::build_stream::<u16>(&device, &config, consumer, Arc::clone(&controls))?
            }
            other => return Err(format!("Unsupported sample format: {other:?}")),
        };

        stream
            .play()
            .map_err(|e| format!("Failed to start audio stream: {e}"))?;

        Ok(Self {
            _stream: stream,
            producer,
            controls,
            sample_rate,
            buffer_capacity,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn buffer_len(&self) -> usize {
        self.producer.len()
    }

    pub fn buffer_capacity(&self) -> usize {
        self.buffer_capacity
    }

    pub fn controls(&self) -> SharedAudioControls {
        Arc::clone(&self.controls)
    }

    pub fn push_samples(&self, samples: &[f32]) {
        let dropped = self.producer.push_samples(samples);
        if dropped > 0 {
            self.controls.add_overflows(dropped);
        }
    }

    pub fn clear_buffer(&self) {
        self.producer.clear();
    }

    pub fn reset_diagnostics(&self) {
        self.controls.reset_diagnostics();
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        consumer: SampleConsumer,
        controls: SharedAudioControls,
    ) -> Result<cpal::Stream, String>
    where
        T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample,
    {
        let channels = config.channels as usize;
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _| {
                    Self::write_output(data, channels, &consumer, &controls)
                },
                |err| eprintln!("Audio stream error: {err}"),
                None,
            )
            .map_err(|e| format!("Failed to build audio stream: {e}"))
    }

    fn write_output<T>(
        output: &mut [T],
        channels: usize,
        consumer: &SampleConsumer,
        controls: &SharedAudioControls,
    ) where
        T: cpal::Sample + cpal::FromSample<f32>,
    {
        let muted = controls.muted();
        let volume = controls.volume();
        let mut underflows = 0_u64;
        let mut last_left = 0.0_f32;
        let mut last_right = 0.0_f32;

        for frame in output.chunks_exact_mut(channels) {
            // Sample buffer stores interleaved [L, R, L, R, ...]; always pop a stereo pair.
            // Since push_samples only ever pushes complete pairs, the queue length is always
            // even and left/right are never swapped.
            let (left, right) = match (consumer.pop_sample(), consumer.pop_sample()) {
                (Some(l), Some(r)) => (l, r),
                (Some(l), None) => { underflows += 1; (l, last_right) },
                _ => { underflows += 1; (last_left, last_right) },
            };
            last_left = left;
            last_right = right;

            let l_out = if muted { 0.0 } else { left * volume };
            let r_out = if muted { 0.0 } else { right * volume };

            for (i, ch) in frame.iter_mut().enumerate() {
                *ch = T::from_sample(if i == 1 { r_out } else { l_out });
            }
        }

        if underflows > 0 {
            controls.add_underflows(underflows);
        }
    }
}
