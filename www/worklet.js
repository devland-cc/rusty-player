class RustyPlayerWorklet extends AudioWorkletProcessor {
  constructor(options) {
    super();

    const { sharedBuffer, channels } = options.processorOptions;
    this.channels = channels || 2;

    const RING_CAPACITY = (sharedBuffer.byteLength - 8) / 4;
    this.RING_CAPACITY = RING_CAPACITY;
    this.writeIdx = new Int32Array(sharedBuffer, 0, 1);
    this.readIdx = new Int32Array(sharedBuffer, 4, 1);
    this.ringData = new Float32Array(sharedBuffer, 8, RING_CAPACITY);
  }

  process(inputs, outputs, parameters) {
    const output = outputs[0];
    if (!output || output.length === 0) return true;

    const numFrames = output[0].length;
    const channels = Math.min(this.channels, output.length);

    const write = Atomics.load(this.writeIdx, 0);
    const read = Atomics.load(this.readIdx, 0);
    const available = (write - read + this.RING_CAPACITY) % this.RING_CAPACITY;
    const needed = numFrames * channels;

    if (available < needed) {
      // Buffer underrun — output silence.
      for (let ch = 0; ch < output.length; ch++) {
        output[ch].fill(0);
      }
      return true;
    }

    // Read interleaved samples and deinterleave to output channels.
    let ri = read;
    for (let frame = 0; frame < numFrames; frame++) {
      for (let ch = 0; ch < channels; ch++) {
        output[ch][frame] = this.ringData[ri % this.RING_CAPACITY];
        ri++;
      }
      // Skip extra channels in ring if output has fewer channels.
      for (let ch = channels; ch < this.channels; ch++) {
        ri++;
      }
    }

    Atomics.store(this.readIdx, 0, ri % this.RING_CAPACITY);

    // Fill any extra output channels with silence.
    for (let ch = channels; ch < output.length; ch++) {
      output[ch].fill(0);
    }

    return true;
  }
}

registerProcessor('rusty-player-worklet', RustyPlayerWorklet);
