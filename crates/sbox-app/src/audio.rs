//! Sound playback: decodes `vsnd_c` payloads (PCM16 / MP3 / Ogg Vorbis) and plays them through the default device.

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use sbox_resource::vsnd;
use std::io::Cursor;

pub struct Audio {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sinks: Vec<Sink>,
}

impl Audio {
    /// Opens the default output device; `None` when there is none (headless machines).
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Self { _stream: stream, handle, sinks: Vec::new() })
    }

    /// Start playing a compiled sound. Returns false if the format is unsupported or decoding fails.
    pub fn play(&mut self, file: &[u8], looping: bool, volume: f32) -> bool {
        let sound = match vsnd::parse(file) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("sound: {e:#}");
                return false;
            }
        };
        let Some(bytes) = sound.to_file_bytes() else {
            log::warn!("sound format {:?} is not supported yet", sound.format);
            return false;
        };
        let dec = match Decoder::new(Cursor::new(bytes)) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("sound decode ({:?}): {e}", sound.format);
                return false;
            }
        };
        let Ok(sink) = Sink::try_new(&self.handle) else { return false };
        sink.set_volume(volume);
        if looping {
            use rodio::Source;
            sink.append(dec.repeat_infinite());
        } else {
            sink.append(dec);
        }
        self.sinks.retain(|s| !s.empty());
        self.sinks.push(sink);
        true
    }

    pub fn is_playing(&self) -> bool {
        self.sinks.iter().any(|s| !s.empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    fn decoded_seconds(path: &str) -> Option<(f32, f32)> {
        let b = std::fs::read(path).ok()?;
        let s = vsnd::parse(&b).unwrap();
        let dec = Decoder::new(Cursor::new(s.to_file_bytes()?)).expect("decoder");
        let (ch, sr) = (dec.channels() as f32, dec.sample_rate() as f32);
        let n = dec.count() as f32;
        Some((n / ch / sr, s.duration))
    }

    #[test]
    fn real_sounds_decode_to_the_header_duration() {
        for path in [
            "E:/sbox-ref/game/addons/menu/Assets/music/menu-avatar.vsnd_c",
            "E:/sbox-ref/game/addons/menu/Assets/music/furniture_shop_loop.vsnd_c",
            "E:/sbox-ref/game/core/sounds/kenney/ui/error_008.vsnd_c",
        ] {
            let Some((got, want)) = decoded_seconds(path) else { continue };
            assert!((got - want).abs() < 0.25, "{path}: decoded {got}s, header says {want}s");
        }
    }
}
