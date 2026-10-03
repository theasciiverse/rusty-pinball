use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use rodio::mixer::Mixer;
use std::io::Cursor;

/// The sounds the game makes, as in the original's `AudioPlayer`.
pub enum SoundTrigger {
    /// Any scoring contact: one of six dings, never the same twice in a row.
    Score,
    /// A rollover lit: up to three notes of the pentatonic scale at once.
    Rollover,
    FlipperUp,
    BallLaunch,
    /// A game message was shown.
    Message,
    /// The game is over; the music starts over.
    Start,
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
static LAUNCH_SOUND:   &[u8] = include_bytes!("../../assets/audio/misc/andBounce2.ogg");
static MESSAGE_SOUND:  &[u8] = include_bytes!("../../assets/audio/misc/message2.ogg");
static START_SOUND:    &[u8] = include_bytes!("../../assets/audio/misc/startup1.ogg");
static ROLLOVER_SOUND: &[u8] = include_bytes!("../../assets/audio/misc/rolloverE.ogg");
static DRUM_MUSIC:     &[u8] = include_bytes!("../../assets/audio/music/drumbassloop.ogg");
static PAD_MUSIC:      &[u8] = include_bytes!("../../assets/audio/music/androidpad.ogg");

/// Playback speeds that turn the rollover's E into the other notes of a pentatonic scale:
/// C, D, E, G, A and the C above.
const ROLLOVER_PITCHES: [f32; 6] = [0.7937008, 0.8908991, 1.0, 1.1892079, 1.3348408, 1.5874025];

/// When the original brings its music in. It is silent at first: the drum loop starts at
/// every 20th scoring hit (if it is not already playing), and the pad at the 10th and then
/// ever less often (the gap grows by 42 each time).
pub struct MusicSchedule {
    hits: u32,
    pad_every: u32,
}

impl MusicSchedule {
    pub fn new() -> Self { Self { hits: 0, pad_every: 10 } }

    pub fn reset(&mut self) { *self = Self::new(); }

    /// Counts a scoring hit; returns whether the drum loop and the pad are due.
    pub fn on_score(&mut self) -> (bool, bool) {
        self.hits += 1;
        let drum = self.hits % 20 == 0;
        let pad = self.hits % self.pad_every == 0;
        if pad { self.pad_every += 42; }
        (drum, pad)
    }
}

/// The next number in 0..1 from a small linear congruential generator.
fn lcg(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / (1u32 << 24) as f32
}

/// A different ding from the last one, from `random` in 0..1.
pub fn next_ding(last: usize, mut random: impl FnMut() -> f32) -> usize {
    loop {
        let pick = ((random() * BUMPER_SOUNDS.len() as f32) as usize).min(BUMPER_SOUNDS.len() - 1);
        if pick != last { return pick; }
    }
}

/// The distinct pitches of up to three random notes, in the order they play.
pub fn rollover_pitch_indices(picks: [usize; 3]) -> Vec<usize> {
    let mut out = vec![picks[0]];
    if picks[1] != picks[0] { out.push(picks[1]); }
    if picks[2] != picks[0] && picks[2] != picks[1] { out.push(picks[2]); }
    out
}

pub struct AudioManager {
    // _sink keeps the OS audio stream alive; must outlive mixer / player.
    _sink:     rodio::stream::MixerDeviceSink,
    mixer:     Mixer,
    drum:      Option<Player>,
    pad:       Option<Player>,
    schedule:  MusicSchedule,
    enabled:   bool,
    last_ding: usize,
    seed:      u32,
}

impl AudioManager {
    /// Returns `None` if no audio device is available.
    pub fn try_new() -> Option<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink().ok()?;
        sink.log_on_drop(false);
        let mixer = sink.mixer().clone();
        Some(Self {
            _sink: sink,
            mixer,
            drum: None,
            pad: None,
            schedule: MusicSchedule::new(),
            enabled: true,
            last_ding: usize::MAX,
            seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(1),
        })
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        for player in [&self.drum, &self.pad].into_iter().flatten() {
            if enabled { player.play(); } else { player.pause(); }
        }
    }

    fn random(&mut self) -> f32 { lcg(&mut self.seed) }

    fn effect(&self, bytes: &'static [u8], volume: f32, speed: f32) {
        let Ok(decoder) = Decoder::new(Cursor::new(bytes)) else { return };
        let player = Player::connect_new(&self.mixer);
        player.append(decoder.speed(speed).amplify(volume));
        player.detach();
    }

    /// Starts a music track once (not looped) unless it is still playing.
    fn start_music(mixer: &Mixer, slot: &mut Option<Player>, bytes: &'static [u8], volume: f32) {
        if slot.as_ref().map_or(false, |p| !p.empty()) { return; }
        let Ok(decoder) = Decoder::new(Cursor::new(bytes)) else { return };
        let player = Player::connect_new(mixer);
        player.set_volume(volume);
        player.append(decoder);
        *slot = Some(player);
    }

    fn reset_music(&mut self) {
        for slot in [&mut self.drum, &mut self.pad] {
            if let Some(player) = slot.take() { player.stop(); }
        }
        self.schedule.reset();
    }

    pub fn play(&mut self, trigger: SoundTrigger) {
        match trigger {
            SoundTrigger::Score => {
                let mut seed = self.seed;
                let ding = next_ding(self.last_ding, || lcg(&mut seed));
                self.seed = seed;
                self.last_ding = ding;
                self.effect(BUMPER_SOUNDS[ding], 0.5, 1.0);
                let (drum, pad) = self.schedule.on_score();
                if self.enabled {
                    if drum { Self::start_music(&self.mixer, &mut self.drum, DRUM_MUSIC, 0.5); }
                    if pad { Self::start_music(&self.mixer, &mut self.pad, PAD_MUSIC, 0.3); }
                }
            }
            SoundTrigger::Rollover => {
                let picks = [(self.random() * 6.0) as usize % 6, (self.random() * 6.0) as usize % 6, (self.random() * 6.0) as usize % 6];
                for i in rollover_pitch_indices(picks) {
                    self.effect(ROLLOVER_SOUND, 0.3, ROLLOVER_PITCHES[i]);
                }
            }
            SoundTrigger::FlipperUp  => self.effect(FLIPPER_SOUND, 1.0, 1.0),
            SoundTrigger::BallLaunch => self.effect(LAUNCH_SOUND, 1.0, 1.0),
            SoundTrigger::Message    => self.effect(MESSAGE_SOUND, 0.66, 1.0),
            SoundTrigger::Start => {
                self.reset_music();
                self.effect(START_SOUND, 0.5, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_music_comes_in_on_the_original_schedule() {
        let mut m = MusicSchedule::new();
        let mut drums = Vec::new();
        let mut pads = Vec::new();
        for hit in 1..=150 {
            let (drum, pad) = m.on_score();
            if drum { drums.push(hit); }
            if pad { pads.push(hit); }
        }
        assert_eq!(drums, vec![20, 40, 60, 80, 100, 120, 140], "the drum loop at every 20th hit");
        assert_eq!(pads, vec![10, 52, 94, 136], "the pad at 10, then whenever the count reaches 52, 94, 136 (the divisor grows by 42 each time)");
        m.reset();
        assert_eq!(m.on_score(), (false, false));
    }

    #[test]
    fn a_ding_is_never_the_same_twice_in_a_row() {
        let mut state = 0u32;
        let mut random = move || { state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (state >> 8) as f32 / (1u32 << 24) as f32 };
        let mut last = usize::MAX;
        for _ in 0..500 {
            let d = next_ding(last, &mut random);
            assert_ne!(d, last);
            assert!(d < 6);
            last = d;
        }
    }

    #[test]
    fn a_rollover_plays_only_distinct_notes() {
        assert_eq!(rollover_pitch_indices([2, 2, 2]), vec![2]);
        assert_eq!(rollover_pitch_indices([1, 1, 4]), vec![1, 4]);
        assert_eq!(rollover_pitch_indices([0, 3, 0]), vec![0, 3]);
        assert_eq!(rollover_pitch_indices([5, 1, 3]), vec![5, 1, 3]);
    }
}
