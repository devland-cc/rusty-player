mod decoder;
mod processor;
mod resampler;
mod vocoder;

use serde::Serialize;
use wasm_bindgen::prelude::*;

use decoder::decode_mp3;
use processor::AudioProcessor;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[derive(Serialize)]
struct TrackInfo {
    sample_rate: u32,
    channels: usize,
    duration_secs: f64,
}

#[wasm_bindgen]
pub struct RustyPlayer {
    processor: AudioProcessor,
}

#[wasm_bindgen]
impl RustyPlayer {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: u32) -> Self {
        Self {
            processor: AudioProcessor::new(sample_rate),
        }
    }

    /// Load MP3 bytes. Returns JSON with track info.
    pub fn load_mp3(&mut self, data: &[u8]) -> Result<JsValue, JsValue> {
        let decoded = decode_mp3(data).map_err(|e| JsValue::from_str(&e))?;

        let info = TrackInfo {
            sample_rate: decoded.sample_rate,
            channels: decoded.channels,
            duration_secs: decoded.duration_secs,
        };

        self.processor
            .load(decoded.samples, decoded.channels, decoded.sample_rate);

        serde_wasm_bindgen::to_value(&info)
            .map_err(|e| JsValue::from_str(&format!("Serialize error: {e}")))
    }

    pub fn play(&mut self) {
        self.processor.play();
    }

    pub fn pause(&mut self) {
        self.processor.pause();
    }

    pub fn seek(&mut self, position_secs: f64) {
        self.processor.seek(position_secs);
    }

    /// Set tempo ratio (1.0 = original, 2.0 = double speed, 0.5 = half speed).
    pub fn set_tempo(&mut self, ratio: f64) {
        self.processor.set_tempo(ratio);
    }

    /// Set pitch shift in semitones (-12 to +12).
    pub fn set_pitch(&mut self, semitones: f64) {
        self.processor.set_pitch(semitones);
    }

    /// Enable/disable Mid-Side processing to preserve stereo image.
    pub fn set_mid_side_mode(&mut self, enabled: bool) {
        self.processor.set_mid_side_mode(enabled);
    }

    pub fn mid_side_mode(&self) -> bool {
        self.processor.mid_side_mode()
    }

    /// Set gain compensation amount (0.0 = none, 1.0 = full).
    pub fn set_gain_comp_amount(&mut self, amount: f64) {
        self.processor.set_gain_comp_amount(amount);
    }

    pub fn gain_comp_amount(&self) -> f64 {
        self.processor.gain_comp_amount()
    }

    /// Process n_frames of audio, returning interleaved f32 samples.
    /// Called from main thread to fill the shared ring buffer.
    pub fn process(&mut self, n_frames: u32) -> Vec<f32> {
        self.processor.fill_output(n_frames as usize)
    }

    pub fn position_secs(&self) -> f64 {
        self.processor.position_secs()
    }

    pub fn duration_secs(&self) -> f64 {
        self.processor.duration_secs()
    }

    pub fn is_loaded(&self) -> bool {
        self.processor.is_loaded()
    }

    pub fn is_playing(&self) -> bool {
        self.processor.is_playing()
    }

    pub fn channels(&self) -> usize {
        self.processor.channels()
    }

    /// Load a test tone (440Hz sine, 5 seconds) to verify audio pipeline.
    pub fn load_test_tone(&mut self) -> JsValue {
        self.processor.load_test_tone(5.0);
        let info = TrackInfo {
            sample_rate: self.processor.sample_rate(),
            channels: 2,
            duration_secs: 5.0,
        };
        serde_wasm_bindgen::to_value(&info).unwrap_or(JsValue::NULL)
    }
}
