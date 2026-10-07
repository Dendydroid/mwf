use super::SAMPLE_RATE;
use anyhow::{anyhow, Context};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::microphone::MicrophoneBuilder;
use rodio::{ChannelCount, SampleRate};
use sherpa_onnx::LinearResampler;
use tracing::info;

const WANTED_SAMPLE_RATE: SampleRate = SampleRate::new(SAMPLE_RATE as u32).unwrap();
const MONO: ChannelCount = ChannelCount::new(1).unwrap();

// The device is read a hundredth of a second at a time
const READS_PER_SECOND: u32 = 100;

/// The default input device as 16 kHz mono, whatever it records in.
pub struct Microphone {
    name: String,
    device: rodio::microphone::Microphone,
    channels: usize,
    frames_per_read: usize,
    // `None` when the device records at 16 kHz itself
    resampler: Option<LinearResampler>,
    // Recorded and resampled, not read yet
    recorded: Vec<f32>,
}

impl Microphone {
    pub fn open() -> anyhow::Result<Self> {
        let name = default_device_name();

        // Asked for what is needed, and given the device's own format where it cannot do that
        let device = MicrophoneBuilder::new()
            .default_device()
            .context("No microphone was found")?
            .default_config()
            .context("The microphone has no usable format")?
            .prefer_sample_rates([WANTED_SAMPLE_RATE])
            .prefer_channel_counts([MONO])
            .open_stream()
            .context("The microphone could not be opened")?;

        let config = *device.config();
        let sample_rate = config.sample_rate.get();

        let resampler = match sample_rate as i32 {
            SAMPLE_RATE => None,
            other => Some(
                LinearResampler::create(other, SAMPLE_RATE)
                    .ok_or_else(|| anyhow!("Cannot resample the microphone's {other} Hz to {SAMPLE_RATE} Hz"))?,
            ),
        };

        info!("Microphone opened: {name}, {sample_rate} Hz, {} channel(s)", config.channel_count);

        Ok(Self {
            name,
            device,
            channels: usize::from(config.channel_count.get()),
            frames_per_read: (sample_rate / READS_PER_SECOND) as usize,
            resampler,
            recorded: Vec::new(),
        })
    }

    /// The device as the system's sound settings name it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The next `samples` samples. Blocks until the microphone has recorded them.
    pub fn read(&mut self, samples: usize) -> anyhow::Result<Vec<f32>> {
        while self.recorded.len() < samples {
            let mono = self.read_from_device()?;

            match &self.resampler {
                Some(resampler) => self.recorded.extend(resampler.resample(&mono, false)),
                None => self.recorded.extend(mono),
            }
        }

        Ok(self.recorded.drain(..samples).collect())
    }

    /// Throws away what was recorded while nobody read it. The device keeps only the first
    /// tenth of a second of that, which is old by now.
    pub fn skip_recorded(&mut self) {
        let (waiting, _) = self.device.size_hint();
        for _ in 0..waiting {
            self.device.next();
        }

        self.recorded.clear();
        if let Some(resampler) = &self.resampler {
            resampler.reset();
        }
    }

    /// A hundredth of a second from the device, its channels mixed into one.
    fn read_from_device(&mut self) -> anyhow::Result<Vec<f32>> {
        let mut mono = Vec::with_capacity(self.frames_per_read);

        for _ in 0..self.frames_per_read {
            let mut frame = 0.0;
            for _ in 0..self.channels {
                frame += self
                    .device
                    .next()
                    .ok_or_else(|| anyhow!("The microphone stopped recording"))?;
            }
            mono.push(frame / self.channels as f32);
        }

        Ok(mono)
    }
}

/// The default input device as the system's sound settings name it: "Microphone (USB Audio)".
fn default_device_name() -> String {
    rodio::cpal::default_host()
        .default_input_device()
        .and_then(|device| device.description().ok())
        .map(|description| match description.driver() {
            Some(driver) => format!("{} ({driver})", description.name()),
            None => description.name().to_string(),
        })
        .unwrap_or_default()
}
