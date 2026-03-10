import init, { RustyPlayer } from '../pkg/rusty_player.js';

let audioCtx = null;
let config = null;
let gainNodeA = null;
let gainNodeB = null;

const CHUNK_FRAMES = 4096;

// --- Deck ---

class Deck {
  constructor(id) {
    this.id = id;
    this.player = null;
    this.gainNode = null;
    this.isPlaying = false;
    this.nextStartTime = 0;
    this.schedulerTimer = null;
    this.trackChannels = 2;
    this.normX = 0.5;
    this.normY = 0.5;
    this.dragging = false;

    // Analysis state.
    this.originalBpm = null;
    this.originalKey = null;
    this.firstBeatSecs = 0;
    this.bpmConfidence = 0;
    this.keyConfidence = 0;

    // Beat grid: array of beat positions in source-time seconds.
    this.beatTimes = [];
    // Local BPM at current playback position (updated in tick loop).
    this.localBpm = null;
    // Current tempo multiplier from XY pad (cached for BPM display).
    this.currentTempo = 1.0;

    // DOM refs.
    this.el = {
      info: document.querySelector(`.deck-info[data-deck="${id}"]`),
      pad: document.querySelector(`.xy-pad[data-deck="${id}"]`),
      dot: document.querySelector(`#deck-${id} .xy-dot`),
      crossH: document.querySelector(`#deck-${id} .crosshair-h`),
      crossV: document.querySelector(`#deck-${id} .crosshair-v`),
      tempoVal: document.querySelector(`.tempo-val[data-deck="${id}"]`),
      pitchVal: document.querySelector(`.pitch-val[data-deck="${id}"]`),
      bpmVal: document.querySelector(`.bpm-val[data-deck="${id}"]`),
      keyVal: document.querySelector(`.key-val[data-deck="${id}"]`),
      beatLed: document.querySelector(`.beat-led[data-deck="${id}"]`),
      timeDisplay: document.querySelector(`.time-display[data-deck="${id}"]`),
    };

    this.bindEvents();
    this.resetXY();
  }

  initPlayer() {
    if (!audioCtx) {
      audioCtx = new AudioContext();
      gainNodeA = audioCtx.createGain();
      gainNodeB = audioCtx.createGain();
      gainNodeA.connect(audioCtx.destination);
      gainNodeB.connect(audioCtx.destination);
      // Initial equal-power at center.
      gainNodeA.gain.value = Math.cos(0.5 * Math.PI / 2);
      gainNodeB.gain.value = Math.sin(0.5 * Math.PI / 2);
    }

    this.gainNode = this.id === 'a' ? gainNodeA : gainNodeB;

    if (!this.player) {
      this.player = new RustyPlayer(audioCtx.sampleRate);
      applyConfig(this.player);
    }
  }

  bindEvents() {
    // File loader.
    const fileInput = document.querySelector(`.deck-file[data-deck="${this.id}"]`);
    fileInput.addEventListener('change', (e) => this.loadFile(e.target.files[0]));

    // XY pad.
    const pad = this.el.pad;
    pad.addEventListener('pointerdown', (e) => {
      this.dragging = true;
      pad.setPointerCapture(e.pointerId);
      this.onPointer(e);
    });
    pad.addEventListener('pointermove', (e) => {
      if (this.dragging) {
        e.preventDefault();
        this.onPointer(e);
      }
    });
    pad.addEventListener('pointerup', () => { this.dragging = false; });
    pad.addEventListener('dblclick', () => { this.resetXY(); });

    // XY reset button.
    document.querySelector(`.btn-reset-xy[data-deck="${this.id}"]`)
      .addEventListener('click', () => this.resetXY());

    // Transport.
    document.querySelector(`.btn-play[data-deck="${this.id}"]`)
      .addEventListener('click', () => this.play());
    document.querySelector(`.btn-pause[data-deck="${this.id}"]`)
      .addEventListener('click', () => this.pause());
  }

  async loadFile(file) {
    if (!file) return;
    this.pause();
    this.initPlayer();

    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const info = this.player.load_mp3(bytes);
      this.trackChannels = info.channels;
      this.el.info.textContent =
        `${file.name} | ${info.channels === 2 ? 'Stereo' : 'Mono'} | ${formatTime(info.duration_secs)}`;
      this.resetXY();

      // Run offline BPM + key analysis.
      try {
        const analysis = this.player.analyze();
        this.originalBpm = analysis.bpm > 0 ? analysis.bpm : null;
        this.originalKey = analysis.key !== '---' ? analysis.key : null;
        this.firstBeatSecs = analysis.first_beat_secs;
        this.bpmConfidence = analysis.bpm_confidence;
        this.keyConfidence = analysis.key_confidence;
        this.beatTimes = analysis.beat_times || [];
        this.localBpm = this.originalBpm;

        this.el.bpmVal.textContent = this.originalBpm !== null
          ? this.originalBpm.toFixed(1) : '---';
        this.el.keyVal.textContent = this.originalKey || '---';

        const bpmStr = this.originalBpm !== null ? `${this.originalBpm.toFixed(1)} BPM` : '';
        const keyStr = this.originalKey || '';
        const sep = bpmStr && keyStr ? ' | ' : '';
        const beatStr = this.beatTimes.length > 0 ? ` | ${this.beatTimes.length} beats` : '';
        setStatus(`Deck ${this.id.toUpperCase()}: loaded${sep ? ' | ' + bpmStr + sep + keyStr : ''}${beatStr}`);
      } catch (err) {
        console.warn(`[Deck ${this.id}] Analysis error:`, err);
        this.originalBpm = null;
        this.originalKey = null;
        this.beatTimes = [];
        this.localBpm = null;
        this.el.bpmVal.textContent = '---';
        this.el.keyVal.textContent = '---';
        setStatus(`Deck ${this.id.toUpperCase()}: loaded`);
      }
    } catch (err) {
      console.error(`[Deck ${this.id}] Load error:`, err);
      setStatus(`Deck ${this.id.toUpperCase()}: ${err}`, 'error');
    }
  }

  onPointer(e) {
    const rect = this.el.pad.getBoundingClientRect();
    this.updateXY(
      (e.clientX - rect.left) / rect.width,
      (e.clientY - rect.top) / rect.height,
    );
  }

  updateXY(nx, ny) {
    this.normX = Math.max(0, Math.min(1, nx));
    this.normY = Math.max(0, Math.min(1, ny));

    // Linear tempo: 0.5x at left, 1.5x at right.
    const tempo = 0.5 + this.normX * 1.0;
    // Pitch: +12 at top, -12 at bottom (one octave each way).
    const semitones = 12 - this.normY * 24;

    this.currentTempo = tempo;

    this.el.dot.style.left = (this.normX * 100) + '%';
    this.el.dot.style.top = (this.normY * 100) + '%';
    this.el.crossH.style.top = (this.normY * 100) + '%';
    this.el.crossV.style.left = (this.normX * 100) + '%';

    if (this.player) {
      this.player.set_tempo(tempo);
      this.player.set_pitch(semitones);
    }

    this.el.tempoVal.textContent = tempo.toFixed(2) + 'x';
    const sign = semitones > 0 ? '+' : '';
    this.el.pitchVal.textContent = sign + semitones.toFixed(1) + ' st';

    // Update displayed BPM: localBPM (or originalBPM fallback) × currentTempo.
    const baseBpm = this.localBpm || this.originalBpm;
    if (baseBpm !== null) {
      this.el.bpmVal.textContent = (baseBpm * tempo).toFixed(1);
    }

    // Update displayed key: transpose original key by pitch semitones.
    if (this.originalKey !== null) {
      this.el.keyVal.textContent = transposeKey(this.originalKey, semitones);
    }
  }

  resetXY() {
    this.updateXY(0.5, 0.5);
  }

  async play() {
    if (!this.player || !this.player.is_loaded()) return;
    if (this.isPlaying) return;

    if (audioCtx.state === 'suspended') {
      await audioCtx.resume();
    }

    this.player.play();
    this.isPlaying = true;
    this.nextStartTime = audioCtx.currentTime + 0.05;
    this.scheduleChunks();
  }

  pause() {
    this.isPlaying = false;
    if (this.schedulerTimer !== null) {
      clearTimeout(this.schedulerTimer);
      this.schedulerTimer = null;
    }
    if (this.player) this.player.pause();
  }

  scheduleChunks() {
    this.schedulerTimer = null;

    if (!this.isPlaying || !this.player || !this.player.is_loaded() || !this.player.is_playing()) {
      if (this.player && !this.player.is_playing() && this.isPlaying) {
        this.isPlaying = false;
      }
      return;
    }

    const sampleRate = audioCtx.sampleRate;
    const scheduleAhead = 0.3;
    const now = audioCtx.currentTime;

    if (this.nextStartTime < now) {
      this.nextStartTime = now + 0.05;
    }

    while (this.nextStartTime < now + scheduleAhead) {
      if (!this.player.is_playing()) break;

      const samples = this.player.process(CHUNK_FRAMES);
      if (!samples || samples.length === 0) break;

      const ch = this.trackChannels;
      const outCh = Math.max(ch, 2);
      const frames = Math.floor(samples.length / ch);
      const buf = audioCtx.createBuffer(outCh, frames, sampleRate);

      for (let c = 0; c < ch; c++) {
        const data = buf.getChannelData(c);
        for (let f = 0; f < frames; f++) {
          data[f] = samples[f * ch + c];
        }
      }
      if (ch === 1 && outCh >= 2) {
        buf.getChannelData(1).set(buf.getChannelData(0));
      }

      const source = audioCtx.createBufferSource();
      source.buffer = buf;
      source.connect(this.gainNode);
      source.start(this.nextStartTime);

      this.nextStartTime += frames / sampleRate;
    }

    this.schedulerTimer = setTimeout(() => this.scheduleChunks(), 50);
  }

  updateTimeDisplay() {
    if (this.player && this.player.is_loaded()) {
      const pos = this.player.position_secs();
      const dur = this.player.duration_secs();
      this.el.timeDisplay.textContent = `${formatTime(pos)} / ${formatTime(dur)}`;
    }
  }

  /// Update local BPM from beat grid near current playback position.
  updateLocalBpm() {
    if (this.beatTimes.length < 2 || !this.player || !this.player.is_loaded() || !this.isPlaying) {
      return;
    }

    const sourcePos = this.player.position_secs();
    const idx = findBeatIndex(this.beatTimes, sourcePos);

    // Average nearby beat intervals (up to 4 intervals on each side).
    const range = 4;
    let sum = 0;
    let count = 0;

    for (let i = Math.max(1, idx - range); i < Math.min(this.beatTimes.length, idx + range); i++) {
      const interval = this.beatTimes[i] - this.beatTimes[i - 1];
      if (interval > 0.2 && interval < 2.0) { // 30–300 BPM range sanity.
        sum += interval;
        count++;
      }
    }

    if (count > 0) {
      this.localBpm = 60.0 / (sum / count);
      // Update BPM display with local BPM × current tempo.
      this.el.bpmVal.textContent = (this.localBpm * this.currentTempo).toFixed(1);
    }
  }

  updateBeatLed() {
    if (!this.player || !this.player.is_loaded() || !this.isPlaying || this.beatTimes.length < 2) {
      this.el.beatLed.style.opacity = '0.15';
      this.el.beatLed.style.boxShadow = 'none';
      return;
    }

    // Source position in seconds (tracks original timeline regardless of tempo/pitch).
    const sourcePos = this.player.position_secs();
    const idx = findBeatIndex(this.beatTimes, sourcePos);

    // Need beats on both sides of current position.
    if (idx === 0 || idx >= this.beatTimes.length) {
      this.el.beatLed.style.opacity = '0.15';
      this.el.beatLed.style.boxShadow = 'none';
      return;
    }

    const prevBeat = this.beatTimes[idx - 1];
    const nextBeat = this.beatTimes[idx];
    const beatInterval = nextBeat - prevBeat;

    if (beatInterval <= 0) {
      this.el.beatLed.style.opacity = '0.15';
      this.el.beatLed.style.boxShadow = 'none';
      return;
    }

    // Phase: 0 at prev beat, 1 at next beat.
    const phase = (sourcePos - prevBeat) / beatInterval;

    // Exponential decay: bright flash at beat, rapid fade.
    const brightness = 0.15 + 0.85 * Math.exp(-phase * 8.0);
    this.el.beatLed.style.opacity = brightness.toFixed(3);

    if (brightness > 0.5) {
      this.el.beatLed.style.boxShadow = `0 0 ${(brightness * 10).toFixed(0)}px #00d4ff`;
    } else {
      this.el.beatLed.style.boxShadow = 'none';
    }
  }
}

// --- Helpers ---

/// Binary search: returns index of first beat >= time.
function findBeatIndex(beatTimes, time) {
  let lo = 0;
  let hi = beatTimes.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (beatTimes[mid] < time) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return lo;
}

const NOTE_NAMES = ['C', 'C#', 'D', 'Eb', 'E', 'F', 'F#', 'G', 'Ab', 'A', 'Bb', 'B'];

function transposeKey(keyStr, semitones) {
  const parts = keyStr.split(' ');
  if (parts.length !== 2) return keyStr;

  const noteName = parts[0];
  const quality = parts[1]; // "major" or "minor"

  const idx = NOTE_NAMES.indexOf(noteName);
  if (idx === -1) return keyStr;

  const shift = Math.round(semitones);
  const newIdx = ((idx + shift) % 12 + 12) % 12;
  return NOTE_NAMES[newIdx] + ' ' + quality;
}

function formatTime(secs) {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

function setStatus(msg, cls) {
  const el = document.getElementById('status');
  el.textContent = msg;
  el.className = cls || '';
}

// --- Config ---

function applyConfig(player) {
  if (!config) return;
  if (config.gain_comp_amount !== undefined) player.set_gain_comp_amount(config.gain_comp_amount);
  if (config.mid_side_mode !== undefined) player.set_mid_side_mode(config.mid_side_mode);
  if (config.phase_lock !== undefined) player.set_phase_lock(config.phase_lock);
  if (config.transient_detect !== undefined) player.set_transient_detect(config.transient_detect);
  if (config.cubic_resampler !== undefined) player.set_cubic_resampler(config.cubic_resampler);
  if (config.soft_limiter !== undefined) player.set_soft_limiter(config.soft_limiter);
  if (config.transient_sensitivity !== undefined) player.set_transient_sensitivity(config.transient_sensitivity);
}

// --- Crossfader ---

document.getElementById('crossfader').addEventListener('input', (e) => {
  const position = parseInt(e.target.value) / 100;
  if (gainNodeA) gainNodeA.gain.value = Math.cos(position * Math.PI / 2);
  if (gainNodeB) gainNodeB.gain.value = Math.sin(position * Math.PI / 2);
});

// --- Init ---

async function initApp() {
  setStatus('Loading WASM...', 'loading');
  try {
    await init();
  } catch (e) {
    setStatus(`WASM load failed: ${e}`, 'error');
    return;
  }

  try {
    config = await fetch('../config/player-defaults.json').then(r => r.json());
  } catch (e) {
    console.warn('[AutoDJ] Config load failed, using built-in defaults:', e);
    config = {};
  }

  const deckA = new Deck('a');
  const deckB = new Deck('b');

  // Display + beat LED + local BPM update loop.
  function tick() {
    deckA.updateTimeDisplay();
    deckB.updateTimeDisplay();
    deckA.updateLocalBpm();
    deckB.updateLocalBpm();
    deckA.updateBeatLed();
    deckB.updateBeatLed();
    requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);

  setStatus('Ready — load MP3 files into each deck');
}

initApp();
