//! Audio output: a cpal stream fed from a shared sample queue. If no output
//! device exists (CI, ssh), everything degrades to silent no-ops.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Cap ~1/3 s of backlog; nominal-rate drift trims here instead of growing.
const QUEUE_CAP: usize = 16_384;

pub struct Audio {
    queue: Arc<Mutex<VecDeque<(f32, f32)>>>,
    _stream: Option<cpal::Stream>,
    pub sample_rate: u32,
}

impl Audio {
    pub fn new() -> Self {
        let queue: Arc<Mutex<VecDeque<(f32, f32)>>> = Arc::default();
        let (stream, sample_rate) = Self::open_stream(queue.clone()).unzip();
        Self {
            queue,
            _stream: stream,
            sample_rate: sample_rate.unwrap_or(48_000),
        }
    }

    fn open_stream(
        queue: Arc<Mutex<VecDeque<(f32, f32)>>>,
    ) -> Option<(cpal::Stream, u32)> {
        let device = cpal::default_host().default_output_device()?;
        let config = device.default_output_config().ok()?;
        let rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let stream = device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut q = queue.lock().unwrap();
                    for frame in data.chunks_mut(channels) {
                        let (l, r) = q.pop_front().unwrap_or((0.0, 0.0));
                        frame[0] = l;
                        if channels > 1 {
                            frame[1] = r;
                        }
                    }
                },
                |_err| {},
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some((stream, rate))
    }

    /// Queue a frame's worth of samples, dropping the oldest past the cap.
    pub fn push(&self, samples: &mut Vec<(f32, f32)>) {
        let mut q = self.queue.lock().unwrap();
        q.extend(samples.drain(..));
        while q.len() > QUEUE_CAP {
            q.pop_front();
        }
    }
}
