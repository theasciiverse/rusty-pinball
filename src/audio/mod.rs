use rodio::{Decoder, DeviceSinkBuilder, Player};
use rodio::mixer::Mixer;
use std::io::Cursor;

pub enum SoundTrigger {
    BumperHit,
    FlipperUp,
    BallLaunch,
    BallDrain,
}

static BUMPER_SOUNDS: &[&[u8]] = &[
    include_bytes!("../../assets/audio/bumper/dinga1.ogg"),
    include_bytes!("../../assets/audio/bumper/dingc1.ogg"),
    include_bytes!("../../assets/audio/bumper/dingc2.ogg"),
    include_bytes!("../../assets/audio/bumper/dingd1.ogg"),
    include_bytes!("../../assets/audio/bumper/dinge1.ogg"),
    include_bytes!("../../assets/audio/bumper/dingg1.ogg"),
];
static FLIPPER_SOUND:  &[u8] = include_bytes!("../../assets/audio/misc/flipper1.ogg");
static LAUNCH_SOUND:   &[u8] = include_bytes!("../../assets/audio/misc/ball1.ogg");
static DRAIN_SOUND:    &[u8] = include_bytes!("../../assets/audio/misc/message2.ogg");
static MUSIC:          &[u8] = include_bytes!("../../assets/audio/music/drumbassloop.ogg");

pub struct AudioManager {
    // _sink keeps the OS audio stream alive; must outlive mixer / player.
    _sink:       rodio::stream::MixerDeviceSink,
    mixer:       Mixer,
    _music:      Player,       // keeps the looping background music alive
    bumper_idx:  usize,
}

impl AudioManager {
    /// Returns `None` if no audio device is available.
    pub fn try_new() -> Option<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink().ok()?;
        sink.log_on_drop(false);

        let mixer = sink.mixer().clone();

        // Start looping background music at reduced volume
        let music = Player::connect_new(&mixer);
        if let Ok(looped) = Decoder::new_looped(Cursor::new(MUSIC)) {
            music.append(looped);
            music.set_volume(0.35);
        }

        Some(Self {
            _sink:      sink,
            mixer,
            _music:     music,
            bumper_idx: 0,
        })
    }

    pub fn play(&mut self, trigger: SoundTrigger) {
        let bytes: &'static [u8] = match trigger {
            SoundTrigger::BumperHit => {
                let b = BUMPER_SOUNDS[self.bumper_idx];
                self.bumper_idx = (self.bumper_idx + 1) % BUMPER_SOUNDS.len();
                b
            }
            SoundTrigger::FlipperUp  => FLIPPER_SOUND,
            SoundTrigger::BallLaunch => LAUNCH_SOUND,
            SoundTrigger::BallDrain  => DRAIN_SOUND,
        };

        if let Ok(player) = rodio::play(&self.mixer, Cursor::new(bytes)) {
            player.detach();
        }
    }
}
