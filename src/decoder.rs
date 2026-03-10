use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: usize,
    pub duration_secs: f64,
}

pub fn decode_mp3(data: &[u8]) -> Result<DecodedAudio, String> {
    let cursor = Cursor::new(data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let format_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &MetadataOptions::default())
        .map_err(|e| format!("Probe error: {e}"))?;

    let mut reader = probed.format;

    let track = reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio track found".to_string())?;

    let track_id = track.id;
    let codec_params = &track.codec_params;

    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params.channels.map(|c| c.count()).unwrap_or(2);

    let duration_secs = codec_params
        .n_frames
        .map(|n| n as f64 / sample_rate as f64)
        .unwrap_or(0.0);

    let mut decoder = symphonia::default::get_codecs()
        .make(codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Codec error: {e}"))?;

    // Pre-allocate based on duration metadata to avoid Vec doubling in WASM.
    let capacity = if duration_secs > 0.0 {
        (duration_secs * sample_rate as f64 * channels as f64) as usize
    } else {
        0
    };
    let mut all_samples = Vec::with_capacity(capacity);
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match reader.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("Read error: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("Decode error: {e}")),
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();

        let buf = sample_buf.get_or_insert_with(|| {
            SampleBuffer::<f32>::new(num_frames as u64, spec)
        });

        if buf.capacity() < num_frames {
            *buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        }

        buf.copy_interleaved_ref(decoded);
        all_samples.extend_from_slice(buf.samples());
    }

    let actual_duration = all_samples.len() as f64 / (sample_rate as f64 * channels as f64);

    Ok(DecodedAudio {
        samples: all_samples,
        sample_rate,
        channels,
        duration_secs: actual_duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_valid_mp3() {
        let data = std::fs::read("tests/fixtures/test_440hz.mp3")
            .expect("test fixture missing — run: ffmpeg -y -f lavfi -i 'sine=frequency=440:duration=2:sample_rate=44100' -ac 2 -b:a 128k tests/fixtures/test_440hz.mp3");
        let result = decode_mp3(&data);
        assert!(result.is_ok(), "decode_mp3 failed: {:?}", result.err());

        let decoded = result.unwrap();
        assert_eq!(decoded.sample_rate, 44100, "expected 44100 Hz sample rate");
        assert_eq!(decoded.channels, 2, "expected stereo");
        assert!(
            (decoded.duration_secs - 2.0).abs() < 0.1,
            "expected ~2s duration, got {:.3}s",
            decoded.duration_secs
        );
        assert!(!decoded.samples.is_empty(), "samples should not be empty");

        // All samples should be in valid f32 audio range.
        let max_abs = decoded.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(
            max_abs <= 1.01,
            "samples exceed [-1, 1] range: max_abs={max_abs}"
        );
        assert!(max_abs > 0.01, "samples are near-silent: max_abs={max_abs}");

        // Verify interleaved layout: total samples = frames * channels.
        assert_eq!(
            decoded.samples.len() % decoded.channels,
            0,
            "sample count must be a multiple of channel count"
        );
    }

    #[test]
    fn test_decode_invalid_data_returns_err() {
        let garbage = vec![0u8; 1024];
        let result = decode_mp3(&garbage);
        assert!(result.is_err(), "garbage data should fail");
    }

    #[test]
    fn test_decode_empty_data_returns_err() {
        let result = decode_mp3(&[]);
        assert!(result.is_err(), "empty data should fail");
    }

    #[test]
    fn test_decode_truncated_mp3_returns_something() {
        // A truncated MP3 should either decode partially or error — not panic.
        let data = std::fs::read("tests/fixtures/test_440hz.mp3").unwrap();
        let truncated = &data[..data.len() / 2];
        let result = decode_mp3(truncated);
        // Either Ok (partial decode) or Err (probe/decode failure) — both acceptable.
        // The important thing is: no panic.
        match &result {
            Ok(d) => assert!(!d.samples.is_empty(), "partial decode should have some samples"),
            Err(e) => println!("truncated MP3 returned error (acceptable): {e}"),
        }
    }
}
