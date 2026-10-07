use anyhow::Context;
use rodio::buffer::SamplesBuffer;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{ChannelCount, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate};
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use tracing::info;

const MONO: ChannelCount = ChannelCount::new(1).unwrap();

// The sound card and the room are still sounding when the last sample has been handed over
const FADE_OUT: Duration = Duration::from_millis(300);

/// The default output device. What is queued on it is played in the order it came.
#[derive(Clone)]
pub struct Speaker {
    player: Rc<Player>,
    // Playing stops when the device is dropped
    _device: Rc<MixerDeviceSink>,
}

impl Speaker {
    pub fn open() -> anyhow::Result<Self> {
        let output = rodio::cpal::default_host()
            .default_output_device()
            .context("No loudspeaker was found")?;

        // As the system's sound settings name it: "Speakers (USB Audio)"
        let name = match output.description() {
            Ok(description) => match description.driver() {
                Some(driver) => format!("{} ({driver})", description.name()),
                None => description.name().to_string(),
            },
            Err(_) => String::new(),
        };

        let mut device = DeviceSinkBuilder::from_device(output)
            .and_then(DeviceSinkBuilder::open_stream)
            .context("The loudspeaker could not be opened")?;
        device.log_on_drop(false);

        info!("Loudspeaker opened: {name}");

        Ok(Self {
            player: Rc::new(Player::connect_new(device.mixer())),
            _device: Rc::new(device),
        })
    }

    /// Queues mono `samples` behind what is being played and returns at once.
    pub fn play(&self, samples: &[f32], sample_rate: SampleRate) {
        self.player.append(SamplesBuffer::new(MONO, sample_rate, samples.to_vec()));
    }

    /// Blocks until everything queued has been heard.
    pub fn wait_until_heard(&self) {
        self.player.sleep_until_end();
        thread::sleep(FADE_OUT);
    }
}
