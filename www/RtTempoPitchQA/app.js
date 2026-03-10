import init, { RustyPlayer } from '../pkg/rusty_player.js';

let player = null;
window._player = null; // Expose for console debugging/toggling features.
let audioCtx = null;
let isPlaying = false;
let trackChannels = 2;
let config = null;

// Scheduled playback state.
let nextStartTime = 0;
let schedulerTimer = null; // Track the active scheduler to prevent duplicates.
const CHUNK_FRAMES = 4096; // ~93ms at 44100Hz

// --- Config ---

function applyConfigToPlayer(p) {
  if (!config) return;
  if (config.gain_comp_amount !== undefined) p.set_gain_comp_amount(config.gain_comp_amount);
  if (config.mid_side_mode !== undefined) p.set_mid_side_mode(config.mid_side_mode);
  if (config.phase_lock !== undefined) p.set_phase_lock(config.phase_lock);
  if (config.transient_detect !== undefined) p.set_transient_detect(config.transient_detect);
  if (config.cubic_resampler !== undefined) p.set_cubic_resampler(config.cubic_resampler);
  if (config.soft_limiter !== undefined) p.set_soft_limiter(config.soft_limiter);
  if (config.transient_sensitivity !== undefined) p.set_transient_sensitivity(config.transient_sensitivity);
}

function applyConfigToUI() {
  if (!config) return;
  const gainPct = Math.round((config.gain_comp_amount ?? 0.35) * 100);
  document.getElementById('gain-slider').value = gainPct;
  document.getElementById('gain-value').textContent = gainPct + '%';
  document.getElementById('btn-ms').className = (config.mid_side_mode ?? true) ? 'toggle-on' : 'toggle-off';
  document.getElementById('btn-phase-lock').className = (config.phase_lock ?? false) ? 'toggle-on' : 'toggle-off';
  document.getElementById('btn-transient').className = (config.transient_detect ?? false) ? 'toggle-on' : 'toggle-off';
  document.getElementById('btn-cubic').className = (config.cubic_resampler ?? false) ? 'toggle-on' : 'toggle-off';
  document.getElementById('btn-limiter').className = (config.soft_limiter ?? false) ? 'toggle-on' : 'toggle-off';
  const sensPct = Math.round((config.transient_sensitivity ?? 0.5) * 100);
  document.getElementById('sens-slider').value = sensPct;
  document.getElementById('sens-value').textContent = sensPct + '%';
  document.getElementById('transient-sens').classList.toggle('hidden', !(config.transient_detect ?? false));
}

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
    console.warn('[rusty] Config load failed, using built-in defaults:', e);
    config = {};
  }

  applyConfigToUI();
  setStatus('Ready — load an MP3 file');
}

// --- File Loading ---

document.getElementById('file-input').addEventListener('change', async (e) => {
  const file = e.target.files[0];
  if (!file) return;

  stopPlayback();
  setStatus(`Loading ${file.name}...`, 'loading');

  try {
    if (!audioCtx) {
      audioCtx = new AudioContext();
      console.log('[rusty] AudioContext sampleRate:', audioCtx.sampleRate);
    }
    if (!player) {
      player = new RustyPlayer(audioCtx.sampleRate);
      applyConfigToPlayer(player);
      window._player = player;
    }

    const arrayBuf = await file.arrayBuffer();
    const bytes = new Uint8Array(arrayBuf);
    const info = player.load_mp3(bytes);
    trackChannels = info.channels;

    console.log('[rusty] Track loaded:', info);

    document.getElementById('track-info').textContent =
      `${file.name} | ${info.sample_rate}Hz | ${info.channels === 2 ? 'Stereo' : 'Mono'} | ${formatTime(info.duration_secs)}`;

    document.getElementById('controls').classList.remove('hidden');
    resetXY();
    setStatus('Ready to play');
  } catch (err) {
    console.error('[rusty] Load error:', err);
    setStatus(`Error: ${err}`, 'error');
  }
});

// --- XY Pad ---

const pad = document.getElementById('xy-pad');
const dot = document.getElementById('xy-dot');
const crosshairH = document.getElementById('crosshair-h');
const crosshairV = document.getElementById('crosshair-v');

let normX = 0.5, normY = 0.5;
let dragging = false;

function updateXY(nx, ny) {
  normX = Math.max(0, Math.min(1, nx));
  normY = Math.max(0, Math.min(1, ny));

  const tempoRatio = 0.5 * Math.pow(4, normX);
  const semitones = 12 - normY * 24;

  dot.style.left = (normX * 100) + '%';
  dot.style.top = (normY * 100) + '%';
  crosshairH.style.top = (normY * 100) + '%';
  crosshairV.style.left = (normX * 100) + '%';

  if (player) {
    player.set_tempo(tempoRatio);
    player.set_pitch(semitones);
  }

  document.getElementById('tempo-display').textContent = tempoRatio.toFixed(2) + 'x';
  const sign = semitones > 0 ? '+' : '';
  document.getElementById('pitch-display').textContent = sign + semitones.toFixed(1) + ' st';
}

function resetXY() {
  updateXY(0.5, 0.5);
}

function onPointerEvent(e) {
  if (!dragging) return;
  e.preventDefault();
  const rect = pad.getBoundingClientRect();
  updateXY(
    (e.clientX - rect.left) / rect.width,
    (e.clientY - rect.top) / rect.height,
  );
}

pad.addEventListener('pointerdown', (e) => {
  dragging = true;
  pad.setPointerCapture(e.pointerId);
  const rect = pad.getBoundingClientRect();
  updateXY(
    (e.clientX - rect.left) / rect.width,
    (e.clientY - rect.top) / rect.height,
  );
});
pad.addEventListener('pointermove', onPointerEvent);
pad.addEventListener('pointerup', () => { dragging = false; });
pad.addEventListener('dblclick', () => { resetXY(); });

resetXY();

// --- Test Tone ---
document.getElementById('btn-test-tone').addEventListener('click', async () => {
  stopPlayback();

  if (!audioCtx) {
    audioCtx = new AudioContext();
    console.log('[rusty] AudioContext sampleRate:', audioCtx.sampleRate);
  }
  if (!player) {
    player = new RustyPlayer(audioCtx.sampleRate);
    window._player = player;
  }

  const info = player.load_test_tone();
  trackChannels = info.channels;
  console.log('[rusty] Test tone loaded:', info);

  document.getElementById('track-info').textContent =
    `Test Tone 440Hz | ${audioCtx.sampleRate}Hz | Stereo | 5s`;
  document.getElementById('controls').classList.remove('hidden');
  resetXY();
  setStatus('Test tone ready — press Play');
});

// --- Scheduled AudioBuffer Playback ---
// Process WASM audio → create AudioBuffer → schedule with precise timing.
// Each chunk is scheduled to start exactly when the previous ends.

function scheduleChunks() {
  // Clear timer reference since we're now executing.
  schedulerTimer = null;

  if (!isPlaying || !player || !player.is_loaded() || !player.is_playing()) {
    if (player && !player.is_playing() && isPlaying) {
      isPlaying = false;
      setStatus('Playback ended');
    }
    return;
  }

  const sampleRate = audioCtx.sampleRate;

  // Schedule enough chunks to stay ahead of current time.
  // Keep at least 300ms of audio scheduled ahead.
  const scheduleAhead = 0.3;
  const now = audioCtx.currentTime;

  // If we've fallen behind, reset timing.
  if (nextStartTime < now) {
    nextStartTime = now + 0.05;
  }

  while (nextStartTime < now + scheduleAhead) {
    if (!player.is_playing()) break;

    const samples = player.process(CHUNK_FRAMES);
    if (!samples || samples.length === 0) break;

    const outChannels = Math.max(trackChannels, 2); // Always output at least stereo
    const frames = Math.floor(samples.length / trackChannels);
    const buf = audioCtx.createBuffer(outChannels, frames, sampleRate);

    // Deinterleave WASM output into AudioBuffer channels.
    for (let ch = 0; ch < trackChannels; ch++) {
      const channelData = buf.getChannelData(ch);
      for (let f = 0; f < frames; f++) {
        channelData[f] = samples[f * trackChannels + ch];
      }
    }
    // If mono, copy to right channel for stereo output.
    if (trackChannels === 1 && outChannels >= 2) {
      buf.getChannelData(1).set(buf.getChannelData(0));
    }

    const source = audioCtx.createBufferSource();
    source.buffer = buf;
    source.connect(audioCtx.destination);
    source.start(nextStartTime);

    nextStartTime += frames / sampleRate;
  }

  // Schedule next pump. Store the timer ID so we can cancel it.
  schedulerTimer = setTimeout(scheduleChunks, 50);
}

// --- Transport ---

function stopPlayback() {
  isPlaying = false;
  if (schedulerTimer !== null) {
    clearTimeout(schedulerTimer);
    schedulerTimer = null;
  }
  if (player) {
    player.pause();
  }
}

document.getElementById('btn-play').addEventListener('click', async () => {
  if (!player || !player.is_loaded()) return;
  if (isPlaying) return; // Already playing — don't start a second loop.

  if (audioCtx.state === 'suspended') {
    await audioCtx.resume();
  }

  player.play();
  isPlaying = true;
  nextStartTime = audioCtx.currentTime + 0.05;
  scheduleChunks();
  setStatus('Playing...');
});

document.getElementById('btn-pause').addEventListener('click', () => {
  if (!player) return;
  stopPlayback();
  setStatus('Paused');
});

document.getElementById('btn-stop').addEventListener('click', () => {
  if (!player) return;
  stopPlayback();
  player.seek(0);
  setStatus('Stopped');
});

document.getElementById('btn-reset').addEventListener('click', () => {
  resetXY();
});

// --- Mid/Side Toggle ---

document.getElementById('btn-ms').addEventListener('click', () => {
  if (!player) return;
  const btn = document.getElementById('btn-ms');
  const enabled = !player.mid_side_mode();
  player.set_mid_side_mode(enabled);
  btn.className = enabled ? 'toggle-on' : 'toggle-off';
});

// --- Gain Compensation Slider ---

document.getElementById('gain-slider').addEventListener('input', (e) => {
  const pct = parseInt(e.target.value);
  document.getElementById('gain-value').textContent = pct + '%';
  if (player) {
    player.set_gain_comp_amount(pct / 100);
    console.log('[rusty] gain_comp_amount set to', pct / 100, '→ readback:', player.gain_comp_amount());
  }
});

// --- Quality Toggles ---

function toggleQualityBtn(btnId, getter, setter) {
  document.getElementById(btnId).addEventListener('click', () => {
    if (!player) return;
    const btn = document.getElementById(btnId);
    const enabled = !getter();
    setter(enabled);
    btn.className = enabled ? 'toggle-on' : 'toggle-off';
  });
}

toggleQualityBtn('btn-phase-lock', () => player.phase_lock(), (v) => player.set_phase_lock(v));
toggleQualityBtn('btn-cubic', () => player.cubic_resampler(), (v) => player.set_cubic_resampler(v));
toggleQualityBtn('btn-limiter', () => player.soft_limiter(), (v) => player.set_soft_limiter(v));

document.getElementById('btn-transient').addEventListener('click', () => {
  if (!player) return;
  const btn = document.getElementById('btn-transient');
  const enabled = !player.transient_detect();
  player.set_transient_detect(enabled);
  btn.className = enabled ? 'toggle-on' : 'toggle-off';
  document.getElementById('transient-sens').classList.toggle('hidden', !enabled);
});

document.getElementById('sens-slider').addEventListener('input', (e) => {
  const pct = parseInt(e.target.value);
  document.getElementById('sens-value').textContent = pct + '%';
  if (player) {
    player.set_transient_sensitivity(pct / 100);
  }
});

// --- Time Display ---

function updateTimeDisplay() {
  if (player && player.is_loaded()) {
    const pos = player.position_secs();
    const dur = player.duration_secs();
    document.getElementById('time-display').textContent =
      `${formatTime(pos)} / ${formatTime(dur)}`;
  }
  requestAnimationFrame(updateTimeDisplay);
}

requestAnimationFrame(updateTimeDisplay);

// --- Helpers ---

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

initApp();
