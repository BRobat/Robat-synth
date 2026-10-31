// Audio thread: runs the Rust engine (WebAssembly) and renders the patch.
// Receives the same edit ops as the UI's model instance and posts meter
// snapshots back for the visuals.

const METER_HZ = 30;

function utf8(str) {
  // TextEncoder isn't available in every AudioWorkletGlobalScope.
  const out = [];
  for (let i = 0; i < str.length; i++) {
    let c = str.codePointAt(i);
    if (c > 0xffff) i++;
    if (c < 0x80) out.push(c);
    else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
    else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
  }
  return out;
}

class RobatProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const { wasm, project } = options.processorOptions;
    const module = new WebAssembly.Module(wasm);
    this.x = new WebAssembly.Instance(module, {}).exports;
    this.x.rb_init(sampleRate, 1);
    this.meterLen = this.x.rb_meter_len();
    this.sinceMeters = 0;
    this.call({ op: "load", project });
    this.port.onmessage = (e) => {
      for (const op of e.data) this.call(op);
    };
  }

  call(op) {
    const bytes = utf8(JSON.stringify(op));
    const ptr = this.x.rb_input(bytes.length);
    new Uint8Array(this.x.memory.buffer, ptr, bytes.length).set(bytes);
    this.x.rb_call(bytes.length);
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const frames = out[0].length;
    const ptr = this.x.rb_process(frames);
    const buf = new Float32Array(this.x.memory.buffer, ptr, frames * 2);
    const l = out[0];
    const r = out[1] || out[0];
    for (let i = 0; i < frames; i++) {
      l[i] = buf[2 * i];
      r[i] = buf[2 * i + 1];
    }
    this.sinceMeters += frames;
    if (this.sinceMeters >= sampleRate / METER_HZ) {
      this.sinceMeters = 0;
      const m = new Float32Array(this.x.memory.buffer, this.x.rb_meters(), this.meterLen).slice();
      this.port.postMessage(m, [m.buffer]);
    }
    return true;
  }
}

registerProcessor("robat", RobatProcessor);
