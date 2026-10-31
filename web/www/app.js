// robat synth: mobile web UI.
//
// A main-thread instance of the Rust engine (WebAssembly, no audio) owns the
// project model. Every edit is applied there first; when it succeeds the same
// op is mirrored to the AudioWorklet instance that makes the sound.

const CELL = { x: 190, y: 104 };
const NODE = { w: 144, h: 58 };
const RIPPLE_SECS = 0.9;
const PULSE_SECS = 0.28;
const UNDO_DEPTH = 100;
const AUTOSAVE_KEY = "robat.autosave";
const PROJECTS_KEY = "robat.projects";
const COLORS = {
  seq: "#46dce6", synth: "#ff46a0", amp: "#aaaab4", delay: "#966eff", out: "#5aff8c",
  filter: "#ffaa3c", drive: "#ff5a46", reverb: "#5a96ff", chorus: "#c878ff",
};
const ACCENT = "#d6ff40";
const ERROR = "#ff5050";
const KIND_HELP = {
  seq: "step grid or pattern → notes",
  synth: "notes → audio · 8 voices, filter",
  amp: "audio → audio · gain, pan",
  filter: "audio → audio · lp/bp/hp/notch",
  drive: "audio → audio · saturate, fold, crush",
  delay: "audio → audio · tempo-synced echo",
  chorus: "audio → audio · chorus / flanger",
  reverb: "audio → audio · room to cathedral",
  out: "audio → speakers",
};
const NOTE_NAMES = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
const noteName = (n) => `${NOTE_NAMES[n % 12]}${Math.floor(n / 12) - 2}`;

const $ = (id) => document.getElementById(id);
const el = (tag, props = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") e.className = v;
    else if (k === "style") e.style.cssText = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) if (kid !== null && kid !== undefined) e.append(kid);
  return e;
};

// ---------------------------------------------------------------- engine --

class Model {
  constructor(exports) {
    this.x = exports;
    this.enc = new TextEncoder();
    this.dec = new TextDecoder();
    this.x.rb_init(48000, 0);
  }
  call(op) {
    const bytes = this.enc.encode(JSON.stringify(op));
    const ptr = this.x.rb_input(bytes.length);
    new Uint8Array(this.x.memory.buffer, ptr, bytes.length).set(bytes);
    const len = this.x.rb_call(bytes.length);
    const out = new Uint8Array(this.x.memory.buffer, this.x.rb_result(), len);
    return JSON.parse(this.dec.decode(out));
  }
}

const state = {
  model: null,
  wasmBytes: null,
  specs: null,
  project: { bpm: 120, nodes: [], edges: [] },
  feedback: new Set(), // "from-to" of cables that close a loop
  selected: null,
  connectFrom: null,
  undo: [],
  playing: false,
  audio: null, // { ctx, node }
  pending: [],
  meters: null,
  stepCursor: {},
  view: { x: 0, y: 0, z: 1 },
  vis: { lastTrig: new Map(), flash: new Map(), ripples: [], pulses: [] },
};

const specOf = (kind) => state.specs.kinds.find((k) => k.name === kind);
const nodeById = (id) => state.project.nodes.find((n) => n.id === id);
const canFeed = (a, b) => {
  const sa = specOf(a), sb = specOf(b);
  return sa.output !== "none" && sa.output === sb.input;
};
const kindOf = (n) => n.kind.toLowerCase(); // serde writes "Seq", "Synth", …
const label = (n) => `${kindOf(n)} ${n.id}`;

// Param curve maths (mirrors engine::model::ParamSpec).
const toNorm = (p, v) => {
  const n = p.curve === "log" ? Math.log(v / p.min) / Math.log(p.max / p.min) : (v - p.min) / (p.max - p.min);
  return Math.min(1, Math.max(0, n));
};
const fromNorm = (p, n) => {
  n = Math.min(1, Math.max(0, n));
  if (p.curve === "log") return p.min * Math.pow(p.max / p.min, n);
  const v = p.min + n * (p.max - p.min);
  return p.curve === "int" ? Math.round(v) : v;
};
const fmt = (p, v) => {
  const lab = p.labels[Math.round(v - p.min)];
  if (lab !== undefined) return lab;
  let s;
  if (p.curve === "int") s = String(Math.round(v));
  else if (Math.abs(v) >= 100) s = v.toFixed(0);
  else if (Math.abs(v) >= 10) s = v.toFixed(1);
  else s = v.toFixed(3);
  return p.unit ? `${s} ${p.unit}` : s;
};

function sendAudio(op) {
  if (!state.audio) return;
  state.pending.push(op);
  if (state.pending.length === 1) {
    queueMicrotask(() => {
      state.audio.node.port.postMessage(state.pending);
      state.pending = [];
    });
  }
}

/** Applies an op to the model and mirrors it to the audio thread. */
function edit(op, { snapshot = true, quiet = false } = {}) {
  const before = JSON.stringify(state.project);
  const r = state.model.call(op);
  if (!r.ok) {
    if (!quiet) toast(r.error, true);
    return r;
  }
  if (snapshot && JSON.stringify(r.project) !== before) pushUndo(before);
  setProject(r);
  sendAudio(op);
  scheduleSave();
  return r;
}

function setProject(r) {
  state.project = r.project;
  state.feedback = new Set((r.feedback || []).map(([a, b]) => `${a}-${b}`));
}
const isLoop = (e) => state.feedback.has(`${e.from}-${e.to}`);
const audioOut = (n) => specOf(kindOf(n)).output === "audio";

function pushUndo(json) {
  state.undo.push(json);
  if (state.undo.length > UNDO_DEPTH) state.undo.shift();
  $("undo").disabled = false;
}

function undo() {
  const prev = state.undo.pop();
  $("undo").disabled = state.undo.length === 0;
  if (!prev) return toast("nothing to undo");
  edit({ op: "restore", project: JSON.parse(prev) }, { snapshot: false });
  if (state.selected !== null && !nodeById(state.selected)) state.selected = null;
  renderSheet();
  toast("undone");
}

let saveTimer = 0;
function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      localStorage.setItem(AUTOSAVE_KEY, JSON.stringify(state.project));
    } catch {}
  }, 400);
}

/** Loads a project (or the demo, for `null`) and stops the transport. */
function loadProject(project, msg) {
  const r = edit(project === null ? { op: "demo" } : { op: "load", project });
  if (!r.ok) return;
  setPlaying(false, { send: false });
  state.selected = null;
  state.connectFrom = null;
  renderSheet();
  renderBanner();
  updateBar();
  fitView();
  if (msg) toast(msg);
}

// ----------------------------------------------------------------- audio --

async function ensureAudio() {
  if (state.audio) {
    if (state.audio.ctx.state !== "running") await state.audio.ctx.resume();
    return true;
  }
  const Ctx = window.AudioContext || window.webkitAudioContext;
  if (!Ctx || !window.AudioWorkletNode) {
    toast("this browser has no AudioWorklet support", true);
    return false;
  }
  // iOS: play through the silent switch where supported.
  try {
    if (navigator.audioSession) navigator.audioSession.type = "playback";
  } catch {}
  const ctx = new Ctx({ latencyHint: "interactive" });
  const resumed = ctx.resume(); // must start inside the tap
  try {
    await ctx.audioWorklet.addModule("worklet.js");
    const node = new AudioWorkletNode(ctx, "robat", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { wasm: state.wasmBytes, project: state.project },
    });
    node.port.onmessage = (e) => (state.meters = e.data);
    node.connect(ctx.destination);
    // From here on every edit is mirrored to the worklet.
    state.audio = { ctx, node };
    await resumed;
    // A suspended context (call, background) comes back on the next tap.
    ctx.onstatechange = () => updateBar();
    return true;
  } catch (err) {
    console.error(err);
    toast(`audio failed: ${err.message || err}`, true);
    return false;
  }
}

async function setPlaying(on, { send = true } = {}) {
  if (on && !(await ensureAudio())) return;
  state.playing = on;
  if (send) edit({ op: "play", on }, { snapshot: false });
  updateBar();
}

// -------------------------------------------------------------- top bar --

function updateBar() {
  const play = $("play");
  play.textContent = state.playing ? "■" : "▶";
  play.classList.toggle("playing", state.playing);
  $("bpm").textContent = Math.round(state.project.bpm * 10) / 10;
  $("undo").disabled = state.undo.length === 0;
}

function nudgeBpm(d) {
  edit({ op: "bpm", bpm: Math.round(state.project.bpm) + d }, { snapshot: false });
  updateBar();
}

// ------------------------------------------------------------ the canvas --

const canvas = $("canvas");
const g = canvas.getContext("2d");
let cw = 0, ch = 0, dpr = 1;

function resize() {
  const r = canvas.getBoundingClientRect();
  dpr = Math.min(window.devicePixelRatio || 1, 3);
  cw = r.width;
  ch = r.height;
  canvas.width = Math.round(cw * dpr);
  canvas.height = Math.round(ch * dpr);
}

const worldOf = (n) => ({ x: n.x * CELL.x, y: n.y * CELL.y });
const toScreen = (wx, wy) => ({ x: (wx - state.view.x) * state.view.z, y: (wy - state.view.y) * state.view.z });
const toWorld = (sx, sy) => ({ x: sx / state.view.z + state.view.x, y: sy / state.view.z + state.view.y });

function fitView() {
  resize();
  const nodes = state.project.nodes;
  if (!nodes.length) {
    state.view = { x: -40, y: -60, z: 1 };
    return;
  }
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const n of nodes) {
    const w = worldOf(n);
    x0 = Math.min(x0, w.x); y0 = Math.min(y0, w.y);
    x1 = Math.max(x1, w.x + NODE.w); y1 = Math.max(y1, w.y + NODE.h);
  }
  const pad = 28;
  const z = Math.min(1.6, Math.max(0.3, Math.min((cw - 2 * pad) / (x1 - x0), (ch - 2 * pad - 70) / (y1 - y0))));
  state.view.z = z;
  state.view.x = (x0 + x1) / 2 - cw / 2 / z;
  state.view.y = (y0 + y1) / 2 - (ch - 70) / 2 / z;
}

function nodeAt(sx, sy) {
  const w = toWorld(sx, sy);
  const slop = 10 / state.view.z;
  for (const n of [...state.project.nodes].reverse()) {
    const p = drag && drag.id === n.id ? drag.pos : worldOf(n);
    if (w.x >= p.x - slop && w.x <= p.x + NODE.w + slop && w.y >= p.y - slop && w.y <= p.y + NODE.h + slop) return n;
  }
  return null;
}

/** A cable as a polyline: a smooth curve forwards, or (for a cable that
 *  closes a loop) a route out, under both nodes and back in. */
function cablePath(pa, pb, loop) {
  const s = { x: pa.x + NODE.w, y: pa.y + NODE.h / 2 }, e = { x: pb.x, y: pb.y + NODE.h / 2 };
  if (loop) {
    const low = Math.max(pa.y, pb.y) + NODE.h + 22;
    return [s, { x: s.x + 22, y: s.y }, { x: s.x + 22, y: low }, { x: e.x - 22, y: low }, { x: e.x - 22, y: e.y }, e];
  }
  const k = Math.max(Math.abs(e.x - s.x) * 0.5, 40);
  const pts = [];
  for (let i = 0; i <= 24; i++) {
    const t = i / 24, u = 1 - t;
    pts.push({
      x: s.x * u * u * u + (s.x + k) * 3 * u * u * t + (e.x - k) * 3 * u * t * t + e.x * t * t * t,
      y: s.y * u * u * u + s.y * 3 * u * u * t + e.y * 3 * u * t * t + e.y * t * t * t,
    });
  }
  return pts;
}

/** The point a fraction `t` of the way along a polyline. */
function along(pts, t) {
  const lens = [];
  let total = 0;
  for (let i = 1; i < pts.length; i++) {
    const l = Math.hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y);
    lens.push(l);
    total += l;
  }
  let d = Math.max(0, Math.min(1, t)) * total;
  for (let i = 0; i < lens.length; i++) {
    if (d <= lens[i] || i === lens.length - 1) {
      const f = lens[i] ? Math.min(1, d / lens[i]) : 0;
      return { x: pts[i].x + (pts[i + 1].x - pts[i].x) * f, y: pts[i].y + (pts[i + 1].y - pts[i].y) * f };
    }
    d -= lens[i];
  }
  return pts[pts.length - 1];
}

function rgba(hex, a) {
  const v = parseInt(hex.slice(1), 16);
  return `rgba(${v >> 16},${(v >> 8) & 255},${v & 255},${Math.max(0, Math.min(1, a))})`;
}

function roundRect(x, y, w, h, r) {
  g.beginPath();
  g.moveTo(x + r, y);
  g.arcTo(x + w, y, x + w, y + h, r);
  g.arcTo(x + w, y + h, x, y + h, r);
  g.arcTo(x, y + h, x, y, r);
  g.arcTo(x, y, x + w, y, r);
  g.closePath();
}

const meterOf = (id) => {
  const m = state.meters;
  if (!m) return { level: 0, trig: 0, pos: 0 };
  const b = 2 + 3 * id;
  return { level: m[b], trig: m[b + 1], pos: m[b + 2] };
};

function updateVisuals(now, dt) {
  const v = state.vis;
  for (const [id, f] of v.flash) v.flash.set(id, Math.max(0, f - dt * 4));
  v.ripples = v.ripples.filter((r) => now - r.t < RIPPLE_SECS);
  v.pulses = v.pulses.filter((p) => now - p.t < PULSE_SECS);
  if (!state.meters) return;
  for (const n of state.project.nodes) {
    const t = meterOf(n.id).trig;
    const last = v.lastTrig.get(n.id);
    v.lastTrig.set(n.id, t);
    if (last === undefined || t === last) continue;
    v.flash.set(n.id, 1);
    if (v.ripples.length < 256) v.ripples.push({ id: n.id, t: now });
    for (const e of state.project.edges) {
      if (e.from === n.id && v.pulses.length < 256) v.pulses.push({ a: e.from, b: e.to, t: now });
    }
  }
}

function summary(n) {
  const p = n.params;
  switch (kindOf(n)) {
    case "seq": {
      if (p[0] >= 1) return n.seq?.pattern || "(empty pattern)";
      const on = n.seq.steps.slice(0, p[2]).filter((s) => s.on).length;
      return `${on}/${p[2]} steps · ${p[1]}/bar`;
    }
    case "synth": {
      const s = specOf("synth").params;
      return `${fmt(s[0], p[0])} · ${fmt(s[8], p[8])}`;
    }
    case "amp": return `gain ${p[0].toFixed(2)} · pan ${p[1].toFixed(2)}`;
    case "delay": return `${p[0]}/16 · fb ${p[1].toFixed(2)}`;
    case "filter": return `${specOf("filter").params[0].labels[p[0]]} · ${fmt(specOf("filter").params[1], p[1])}`;
    case "drive": return `${specOf("drive").params[0].labels[p[0]]} · ${p[1].toFixed(2)}${p[2] < 16 ? ` · ${p[2]} bit` : ""}`;
    case "reverb": return `size ${p[0].toFixed(2)} · mix ${p[3].toFixed(2)}`;
    case "chorus": return `${fmt(specOf("chorus").params[0], p[0])} · mix ${p[4].toFixed(2)}`;
    case "out": return `ch ${specOf("out").params[0].labels[p[0]]} · ${p[1].toFixed(2)}`;
  }
  return "";
}

function drawNode(n, pos, now) {
  const color = COLORS[kindOf(n)];
  const sel = state.selected === n.id;
  const m = meterOf(n.id);
  const flash = state.vis.flash.get(n.id) || 0;
  const level = Math.min(1, m.level * 3);
  const { x, y } = pos;
  const target = state.connectFrom !== null && state.connectFrom !== n.id && canFeed(kindOf(nodeById(state.connectFrom)), kindOf(n));

  g.save();
  if (n.muted) g.globalAlpha = 0.4;
  // glow from level and triggers
  const glow = Math.max(level, flash * 0.8);
  if (glow > 0.02) {
    g.shadowColor = rgba(color, 0.9);
    g.shadowBlur = 8 + 30 * glow;
  }
  roundRect(x, y, NODE.w, NODE.h, 10);
  g.fillStyle = "#101013";
  g.fill();
  g.shadowBlur = 0;
  g.fillStyle = rgba(color, 0.06 + 0.22 * flash);
  g.fill();
  g.lineWidth = sel ? 2.5 : 1.5;
  g.strokeStyle = sel ? ACCENT : target ? rgba(ACCENT, 0.5 + 0.5 * Math.sin(now * 6)) : rgba(color, 0.55 + 0.45 * flash);
  g.stroke();

  // kind strip
  g.fillStyle = color;
  roundRect(x + 8, y + 10, 4, NODE.h - 20, 2);
  g.fill();

  g.fillStyle = "#f0f0f0";
  g.font = "600 14px ui-monospace, Menlo, monospace";
  g.textBaseline = "alphabetic";
  g.fillText(label(n), x + 20, y + 24);
  if (n.muted) {
    g.fillStyle = ERROR;
    g.font = "11px ui-monospace, Menlo, monospace";
    g.fillText("muted", x + NODE.w - 46, y + 24);
  }
  g.fillStyle = "#8a8a94";
  g.font = "11px ui-monospace, Menlo, monospace";
  let sum = summary(n);
  while (sum.length > 4 && g.measureText(sum).width > NODE.w - 28) sum = sum.slice(0, -2) + "…";
  g.fillText(sum, x + 20, y + 42);

  // sequencer playhead / level bar
  if (kindOf(n) === "seq") {
    const len = n.params[2];
    const w = (NODE.w - 28) / len;
    for (let i = 0; i < len; i++) {
      const on = n.params[0] < 1 && n.seq.steps[i].on;
      const head = state.playing && i === m.pos;
      g.fillStyle = head ? ACCENT : on ? rgba(color, 0.9) : "#26262c";
      g.fillRect(x + 20 + i * w, y + NODE.h - 9, Math.max(1, w - 1), 3);
    }
  } else {
    g.fillStyle = "#1c1c21";
    g.fillRect(x + 20, y + NODE.h - 9, NODE.w - 28, 3);
    g.fillStyle = color;
    g.fillRect(x + 20, y + NODE.h - 9, (NODE.w - 28) * level, 3);
  }
  // ports
  const spec = specOf(kindOf(n));
  g.fillStyle = "#5f5f69";
  if (spec.input !== "none") { g.beginPath(); g.arc(x, y + NODE.h / 2, 4, 0, 7); g.fill(); }
  if (spec.output !== "none") { g.beginPath(); g.arc(x + NODE.w, y + NODE.h / 2, 4, 0, 7); g.fill(); }
  g.restore();
}

function draw(now) {
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.fillStyle = "#0a0a0c";
  g.fillRect(0, 0, cw, ch);

  const z = state.view.z;
  // dot grid at cell corners
  g.fillStyle = "#1c1c21";
  const tl = toWorld(0, 0), br = toWorld(cw, ch);
  const sx = Math.floor(tl.x / (CELL.x / 2)), ex = Math.ceil(br.x / (CELL.x / 2));
  const sy = Math.floor(tl.y / (CELL.y / 2)), ey = Math.ceil(br.y / (CELL.y / 2));
  if ((ex - sx) * (ey - sy) < 6000) {
    for (let i = sx; i <= ex; i++) for (let j = sy; j <= ey; j++) {
      const p = toScreen(i * CELL.x / 2 - 23, j * CELL.y / 2 - 23);
      g.fillRect(p.x - 1, p.y - 1, 2, 2);
    }
  }

  g.setTransform(dpr * z, 0, 0, dpr * z, -state.view.x * z * dpr, -state.view.y * z * dpr);
  const posOf = (n) => (drag && drag.id === n.id && drag.moved ? drag.pos : worldOf(n));

  // cables
  g.lineJoin = "round";
  for (const edge of state.project.edges) {
    const na = nodeById(edge.from), nb = nodeById(edge.to);
    if (!na || !nb) continue;
    const loop = isLoop(edge);
    const pts = cablePath(posOf(na), posOf(nb), loop);
    g.beginPath();
    g.moveTo(pts[0].x, pts[0].y);
    for (const q of pts.slice(1)) g.lineTo(q.x, q.y);
    g.lineWidth = 1 + Math.min(2, edge.gain) * 1.2;
    g.setLineDash(loop ? [7, 6] : []);
    g.strokeStyle = rgba(COLORS[kindOf(na)], na.muted ? 0.15 : 0.45);
    g.stroke();
    g.setLineDash([]);
    if (Math.abs(edge.gain - 1) > 0.005 && audioOut(na)) {
      const m = along(pts, 0.5);
      const text = `×${edge.gain.toFixed(2)}`;
      g.font = "11px ui-monospace, Menlo, monospace";
      const w = g.measureText(text).width + 8;
      g.fillStyle = "#0a0a0c";
      g.fillRect(m.x - w / 2, m.y - 8, w, 16);
      g.fillStyle = COLORS[kindOf(na)];
      g.textAlign = "center";
      g.textBaseline = "middle";
      g.fillText(text, m.x, m.y);
      g.textAlign = "start";
      g.textBaseline = "alphabetic";
    }
  }
  // pulses running along cables
  for (const p of state.vis.pulses) {
    const na = nodeById(p.a), nb = nodeById(p.b);
    if (!na || !nb) continue;
    const t = (now - p.t) / PULSE_SECS;
    const q = along(cablePath(posOf(na), posOf(nb), state.feedback.has(`${p.a}-${p.b}`)), t);
    g.fillStyle = rgba(COLORS[kindOf(na)], 1 - t * 0.6);
    g.beginPath();
    g.arc(q.x, q.y, 4.5, 0, 7);
    g.fill();
  }
  // ripples
  for (const r of state.vis.ripples) {
    const n = nodeById(r.id);
    if (!n) continue;
    const p = posOf(n);
    const t = (now - r.t) / RIPPLE_SECS;
    const grow = 30 * t;
    roundRect(p.x - grow, p.y - grow, NODE.w + 2 * grow, NODE.h + 2 * grow, 10 + grow);
    g.lineWidth = 1.5;
    g.strokeStyle = rgba(COLORS[kindOf(n)], 0.5 * (1 - t));
    g.stroke();
  }
  // drag target cell
  if (drag && drag.moved) {
    const c = snapCell(drag.pos);
    roundRect(c.x * CELL.x, c.y * CELL.y, NODE.w, NODE.h, 10);
    g.setLineDash([6, 6]);
    g.lineWidth = 1.5;
    g.strokeStyle = rgba(ACCENT, 0.6);
    g.stroke();
    g.setLineDash([]);
  }
  for (const n of state.project.nodes) if (!(drag && drag.id === n.id)) drawNode(n, posOf(n), now);
  if (drag) {
    const n = nodeById(drag.id);
    if (n) drawNode(n, posOf(n), now);
  }

  if (!state.project.nodes.length) {
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = "#5f5f69";
    g.font = "14px ui-monospace, Menlo, monospace";
    g.textAlign = "center";
    g.fillText("empty patch: tap + to add a node", cw / 2, ch / 2 - 10);
    g.fillText("or ☰ → demo", cw / 2, ch / 2 + 12);
    g.textAlign = "start";
  }
}

let lastFrame = 0;
let stepHeadSeen = -1;
function frame(ms) {
  const now = ms / 1000;
  const dt = Math.min(0.1, now - lastFrame);
  lastFrame = now;
  updateVisuals(now, dt);
  draw(now);
  if (state.meters) {
    const cyc = state.meters[0];
    $("cycle").textContent = state.playing ? `${Math.floor(cyc) + 1}.${Math.floor((cyc % 1) * 4) + 1}` : "stopped";
  }
  // keep the open sequencer's playhead live
  const n = state.selected !== null && nodeById(state.selected);
  if (n && kindOf(n) === "seq") {
    const head = state.playing ? meterOf(n.id).pos : -1;
    if (head !== stepHeadSeen) {
      stepHeadSeen = head;
      document.querySelectorAll(".step").forEach((s, i) => s.classList.toggle("head", i === head));
    }
  }
  requestAnimationFrame(frame);
}

// ---------------------------------------------------- canvas gestures --

const pointers = new Map();
let drag = null; // { id, start, pos, offset, moved }
let pan = null; // { sx, sy, vx, vy }
let pinch = null; // { dist, z, mid, world }

function snapCell(pos) {
  return { x: Math.round(pos.x / CELL.x), y: Math.round(pos.y / CELL.y) };
}

canvas.addEventListener("pointerdown", (e) => {
  canvas.setPointerCapture(e.pointerId);
  const r = canvas.getBoundingClientRect();
  const p = { x: e.clientX - r.left, y: e.clientY - r.top };
  pointers.set(e.pointerId, p);
  if (pointers.size === 2) {
    drag = null;
    pan = null;
    const [a, b] = [...pointers.values()];
    const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
    pinch = { dist: Math.hypot(a.x - b.x, a.y - b.y), z: state.view.z, world: toWorld(mid.x, mid.y) };
    return;
  }
  if (pointers.size > 2) return;
  const n = nodeAt(p.x, p.y);
  if (n) {
    const w = toWorld(p.x, p.y), wp = worldOf(n);
    drag = { id: n.id, start: p, pos: { ...wp }, offset: { x: w.x - wp.x, y: w.y - wp.y }, moved: false };
  } else {
    pan = { start: p, vx: state.view.x, vy: state.view.y, moved: false };
  }
});

canvas.addEventListener("pointermove", (e) => {
  if (!pointers.has(e.pointerId)) return;
  const r = canvas.getBoundingClientRect();
  const p = { x: e.clientX - r.left, y: e.clientY - r.top };
  pointers.set(e.pointerId, p);
  if (pinch && pointers.size >= 2) {
    const [a, b] = [...pointers.values()];
    const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
    const z = Math.min(2.5, Math.max(0.25, pinch.z * Math.hypot(a.x - b.x, a.y - b.y) / pinch.dist));
    state.view.z = z;
    state.view.x = pinch.world.x - mid.x / z;
    state.view.y = pinch.world.y - mid.y / z;
    return;
  }
  if (drag) {
    if (!drag.moved && Math.hypot(p.x - drag.start.x, p.y - drag.start.y) > 8) drag.moved = true;
    if (drag.moved) {
      const w = toWorld(p.x, p.y);
      drag.pos = { x: w.x - drag.offset.x, y: w.y - drag.offset.y };
    }
  } else if (pan) {
    if (!pan.moved && Math.hypot(p.x - pan.start.x, p.y - pan.start.y) > 8) pan.moved = true;
    state.view.x = pan.vx - (p.x - pan.start.x) / state.view.z;
    state.view.y = pan.vy - (p.y - pan.start.y) / state.view.z;
  }
});

function endPointer(e) {
  if (!pointers.has(e.pointerId)) return;
  pointers.delete(e.pointerId);
  if (pinch) {
    if (pointers.size < 2) pinch = null;
    return;
  }
  if (drag) {
    const d = drag;
    drag = null;
    if (d.moved) moveNode(d.id, snapCell(d.pos));
    else tapNode(d.id);
  } else if (pan) {
    if (!pan.moved && e.type === "pointerup") tapEmpty();
    pan = null;
  }
}
canvas.addEventListener("pointerup", endPointer);
// Taps open the inspector under the finger; stop the browser's follow-up
// "ghost" click from landing on whatever button appears there.
canvas.addEventListener("touchend", (e) => e.preventDefault(), { passive: false });
canvas.addEventListener("pointercancel", endPointer);
canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const r = canvas.getBoundingClientRect();
  const p = { x: e.clientX - r.left, y: e.clientY - r.top };
  if (e.ctrlKey || e.metaKey) {
    const w = toWorld(p.x, p.y);
    const z = Math.min(2.5, Math.max(0.25, state.view.z * Math.exp(-e.deltaY * 0.01)));
    state.view.z = z;
    state.view.x = w.x - p.x / z;
    state.view.y = w.y - p.y / z;
  } else {
    state.view.x += e.deltaX / state.view.z;
    state.view.y += e.deltaY / state.view.z;
  }
}, { passive: false });

function moveNode(id, cell) {
  const n = nodeById(id);
  if (!n || (n.x === cell.x && n.y === cell.y)) return;
  const other = state.project.nodes.find((o) => o.id !== id && o.x === cell.x && o.y === cell.y);
  const before = JSON.stringify(state.project);
  if (other) edit({ op: "move", id: other.id, x: n.x, y: n.y }, { snapshot: false });
  edit({ op: "move", id, x: cell.x, y: cell.y }, { snapshot: false });
  pushUndo(before);
}

function tapNode(id) {
  if (state.connectFrom !== null) {
    const from = state.connectFrom;
    if (id === from && !canFeed(kindOf(nodeById(id)), kindOf(nodeById(id)))) return cancelConnect();
    const r = edit({ op: "connect", from, to: id });
    if (r.ok) {
      toast(`${label(nodeById(from))} → ${label(nodeById(id))}`);
      state.connectFrom = null;
      renderBanner();
      select(id);
    }
    return;
  }
  select(id);
}

function tapEmpty() {
  if (state.connectFrom !== null) return cancelConnect();
  select(null);
}

function select(id) {
  state.selected = id;
  renderSheet();
}

function startConnect() {
  if (state.selected === null) return;
  const n = nodeById(state.selected);
  if (specOf(kindOf(n)).output === "none") return toast("out nodes have no output", true);
  state.connectFrom = state.selected;
  renderBanner();
}

function cancelConnect() {
  state.connectFrom = null;
  renderBanner();
}

function renderBanner() {
  const b = $("banner");
  b.hidden = state.connectFrom === null;
  b.replaceChildren();
  if (state.connectFrom === null) return;
  const n = nodeById(state.connectFrom);
  const selfOk = canFeed(kindOf(n), kindOf(n));
  b.append(el("span", {}, `connect ${label(n)} → tap a target${selfOk ? " (itself = loop)" : ""}`), el("button", { onclick: cancelConnect }, "cancel"));
}

// ---------------------------------------------------------- inspector --

function freeCellNear(x, y) {
  for (let dy = 0; ; dy++) if (!state.project.nodes.some((n) => n.x === x && n.y === y + dy)) return { x, y: y + dy };
}

function addNode(kind) {
  closeOverlay();
  const sel = state.selected !== null ? nodeById(state.selected) : null;
  let cell;
  if (sel) cell = freeCellNear(sel.x + 1, sel.y);
  else {
    const c = snapCell(toWorld(cw / 2 - NODE.w / 2, ch / 3));
    cell = freeCellNear(c.x, c.y);
  }
  const before = JSON.stringify(state.project);
  const r = edit({ op: "add_node", kind, x: cell.x, y: cell.y }, { snapshot: false });
  if (!r.ok) return;
  pushUndo(before);
  if (sel) {
    const c = edit({ op: "connect", from: sel.id, to: r.id }, { snapshot: false, quiet: true });
    if (c.ok) toast(`${label(sel)} → ${kind} ${r.id}`);
  }
  select(r.id);
  ensureVisible(nodeById(r.id));
}

function ensureVisible(n) {
  const p = toScreen(worldOf(n).x, worldOf(n).y);
  const z = state.view.z;
  if (p.x < 10 || p.y < 10 || p.x + NODE.w * z > cw - 10 || p.y + NODE.h * z > ch - 10) {
    state.view.x = worldOf(n).x + NODE.w / 2 - cw / 2 / z;
    state.view.y = worldOf(n).y + NODE.h / 2 - ch / 2 / z;
  }
}

function duplicate() {
  const n = nodeById(state.selected);
  const c = freeCellNear(n.x, n.y + 1);
  const r = edit({ op: "duplicate", id: n.id, x: c.x, y: c.y });
  if (r.ok) {
    select(r.id);
    toast(`copied to ${label(nodeById(r.id))}`);
  }
}

function remove() {
  const n = nodeById(state.selected);
  edit({ op: "remove", id: n.id });
  state.selected = null;
  renderSheet();
  toast(`deleted ${label(n)} · ↶ to undo`);
}

function paramRow(n, idx, p) {
  const v = n.params[idx];
  const color = COLORS[kindOf(n)];
  const nameEl = el("div", { class: "name" }, p.name);
  if (p.labels.length) {
    return el("div", { class: "param" }, nameEl,
      el("div", { class: "segments" }, p.labels.map((lab, i) => el("button", {
        class: Math.round(v - p.min) === i ? "on" : "",
        onclick: () => { edit({ op: "param", id: n.id, idx, value: p.min + i }); renderSheet(); },
      }, lab))));
  }
  const valueBtn = el("button", { class: "value" }, fmt(p, v));
  const range = el("input", { type: "range", min: 0, max: 1000, step: 1, value: Math.round(toNorm(p, v) * 1000), style: `--k:${color}` });
  const paint = () => range.style.setProperty("--fill", `${range.value / 10}%`);
  paint();
  let snapped = false;
  range.addEventListener("pointerdown", () => (snapped = false));
  range.addEventListener("input", () => {
    if (!snapped) { pushUndo(JSON.stringify(state.project)); snapped = true; }
    const value = fromNorm(p, range.value / 1000);
    edit({ op: "param", id: n.id, idx, value }, { snapshot: false });
    valueBtn.textContent = fmt(p, nodeById(n.id).params[idx]);
    paint();
  });
  range.addEventListener("change", () => {
    snapped = false;
    if (kindOf(n) === "seq" && idx === 2) renderSheet(); // length changes the grid
  });
  valueBtn.addEventListener("click", () => {
    const s = prompt(`${p.name} (${p.min}–${p.max}${p.unit ? " " + p.unit : ""}), empty = default ${p.default}`, String(+nodeById(n.id).params[idx].toFixed(4)));
    if (s === null) return;
    const value = s.trim() === "" ? p.default : parseFloat(s);
    if (!Number.isFinite(value)) return toast("not a number", true);
    edit({ op: "param", id: n.id, idx, value });
    renderSheet();
  });
  return el("div", { class: "param" }, nameEl, range, valueBtn);
}

function stepGrid(n) {
  const len = n.params[2];
  const steps = n.seq.steps;
  let cur = Math.min(state.stepCursor[n.id] ?? 0, len - 1);
  const setSteps = (f) => {
    const copy = steps.map((s) => ({ ...s }));
    f(copy);
    edit({ op: "steps", id: n.id, steps: copy });
    renderSheet();
  };
  const head = state.playing ? meterOf(n.id).pos : -1;
  const grid = el("div", { class: "steps" }, Array.from({ length: len }, (_, i) => {
    const s = steps[i];
    return el("button", {
      class: ["step", s.on && "on", i === cur && "cursor", i % 4 === 0 && "beat", i === head && "head"].filter(Boolean).join(" "),
      style: s.on ? `opacity:${0.45 + 0.55 * s.vel}` : "",
      onclick: () => {
        state.stepCursor[n.id] = i;
        setSteps((st) => (st[i].on = !st[i].on));
      },
    }, s.on ? noteName(s.note) : String(i + 1));
  }));
  const s = steps[cur];
  const bump = (d) => setSteps((st) => { st[cur].note = Math.max(0, Math.min(127, st[cur].note + d)); st[cur].on = true; });
  const vel = () => setSteps((st) => { const v = st[cur].vel; st[cur].vel = v < 0.5 ? 0.6 : v < 0.7 ? 0.8 : v < 0.9 ? 1.0 : 0.4; });
  const tools = el("div", { class: "step-tools" },
    el("button", { onclick: () => bump(-12) }, "oct −"),
    el("div", { class: "note" }, `${cur + 1}: ${noteName(s.note)}`),
    el("button", { onclick: () => bump(12) }, "oct +"),
    el("button", { onclick: () => bump(-1) }, "note −"),
    el("button", { onclick: vel }, `vel ${s.vel.toFixed(1)}`),
    el("button", { onclick: () => setSteps((st) => (st[cur].on = !st[cur].on)) }, s.on ? "off" : "on"),
    el("button", { onclick: () => bump(1) }, "note +"),
    el("button", { onclick: () => setSteps((st) => { const a = st.slice(0, len); a.push(a.shift()); st.splice(0, len, ...a); }) }, "◀ rot"),
    el("button", { onclick: () => setSteps((st) => { const a = st.slice(0, len); a.unshift(a.pop()); st.splice(0, len, ...a); }) }, "rot ▶"),
    el("button", { onclick: () => { if (cur + 1 < len) { state.stepCursor[n.id] = cur + 1; setSteps((st) => (st[cur + 1] = { ...st[cur] })); } } }, "copy →"),
    el("button", { class: "danger", onclick: () => setSteps((st) => st.slice(0, len).forEach((x) => (x.on = false))) }, "clear"),
  );
  return [grid, tools];
}

function patternEditor(n) {
  const ta = el("textarea", { autocapitalize: "off", autocomplete: "off", autocorrect: "off", spellcheck: "false", placeholder: "c1 ~ c1 [~ c2] <eb1 g0>" });
  ta.value = n.seq.pattern;
  const msg = el("div", { class: "err" });
  const apply = () => {
    const r = edit({ op: "pattern", id: n.id, src: ta.value });
    if (r.patternError) {
      msg.className = "err";
      msg.textContent = `${r.patternError.text} (old pattern keeps playing)`;
    } else {
      msg.className = "err ok";
      msg.textContent = "✓ takes over at the next bar";
    }
  };
  ta.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey || !e.shiftKey)) { e.preventDefault(); apply(); ta.blur(); }
  });
  const insert = (t) => {
    const a = ta.selectionStart ?? ta.value.length, b = ta.selectionEnd ?? a;
    ta.value = ta.value.slice(0, a) + t + ta.value.slice(b);
    ta.focus();
    ta.setSelectionRange(a + t.length - (t.length === 2 ? 1 : 0), a + t.length - (t.length === 2 ? 1 : 0));
  };
  return el("div", { class: "pattern" }, ta,
    el("div", { class: "row" }, ["~", "[]", "<>", "*", "!", ","].map((t) => el("button", { onpointerdown: (e) => e.preventDefault(), onclick: () => insert(t) }, t))),
    el("div", { class: "row" }, el("button", { class: "on", onclick: apply }, "apply"), el("button", { onclick: showHelp }, "syntax")),
    msg);
}

/** One cable in the inspector: name, loop mark, disconnect, and a level
 *  slider for audio cables coming into this node. */
function cableRow(n, e) {
  const a = nodeById(e.from), b = nodeById(e.to);
  const loop = isLoop(e);
  const name = `${label(a)} → ${e.from === e.to ? "itself" : label(b)}${loop ? "  ↺ loop" : ""}`;
  const row = el("div", { class: "link" },
    el("span", {}, name),
    el("button", { "aria-label": "disconnect", onclick: () => { edit({ op: "disconnect", from: e.from, to: e.to }); renderSheet(); } }, "✕"));
  if (e.to !== n.id || !audioOut(a)) return row;
  const p = { name: "level", min: 0, max: 2, default: 1, curve: "lin", unit: "", labels: [] };
  const valueBtn = el("button", { class: "value" }, fmt(p, e.gain));
  const range = el("input", { type: "range", min: 0, max: 1000, step: 1, value: Math.round(toNorm(p, e.gain) * 1000), style: `--k:${COLORS[kindOf(a)]}` });
  const paint = () => range.style.setProperty("--fill", `${range.value / 10}%`);
  paint();
  let snapped = false;
  range.addEventListener("input", () => {
    if (!snapped) { pushUndo(JSON.stringify(state.project)); snapped = true; }
    const gain = fromNorm(p, range.value / 1000);
    edit({ op: "edge_gain", from: e.from, to: e.to, gain }, { snapshot: false });
    valueBtn.textContent = fmt(p, gain);
    paint();
  });
  range.addEventListener("change", () => (snapped = false));
  valueBtn.addEventListener("click", () => {
    const s = prompt("cable level (0–2), empty = 1", String(+e.gain.toFixed(3)));
    if (s === null) return;
    const gain = s.trim() === "" ? 1 : parseFloat(s);
    if (!Number.isFinite(gain)) return toast("not a number", true);
    edit({ op: "edge_gain", from: e.from, to: e.to, gain });
    renderSheet();
  });
  return el("div", { class: "cable" }, row, el("div", { class: "param" }, el("div", { class: "name" }, "level"), range, valueBtn));
}

function renderSheet() {
  const sheet = $("sheet");
  const body = $("sheet-body");
  const n = state.selected !== null ? nodeById(state.selected) : null;
  document.body.classList.toggle("sheet-open", !!n);
  if (!n) {
    sheet.hidden = true;
    delete sheet.dataset.node;
    requestAnimationFrame(resize);
    return;
  }
  const keepScroll = sheet.hidden ? 0 : body.scrollTop;
  const active = document.activeElement;
  if (active && active.tagName === "TEXTAREA" && body.contains(active)) return; // don't clobber typing
  sheet.hidden = false;
  const spec = specOf(kindOf(n));
  const color = COLORS[kindOf(n)];
  const kids = [
    el("div", { class: "node-head" },
      el("div", { class: "title" }, el("span", { class: "swatch", style: `background:${color}` }), label(n)),
      el("button", { onclick: () => select(null), "aria-label": "close" }, "✕")),
    el("div", { class: "actions" },
      spec.output !== "none" ? el("button", { onclick: startConnect }, "connect →") : null,
      el("button", { class: n.muted ? "on" : "", onclick: () => { edit({ op: "mute", id: n.id }); renderSheet(); } }, n.muted ? "muted" : "mute"),
      el("button", { onclick: duplicate }, "copy"),
      el("button", { class: "danger", onclick: remove }, "delete")),
  ];
  if (kindOf(n) === "seq") {
    const patternMode = n.params[0] >= 1;
    kids.push(paramRow(n, 0, spec.params[0]));
    kids.push(el("h3", {}, patternMode ? "pattern" : "steps"));
    kids.push(...(patternMode ? [patternEditor(n)] : stepGrid(n)));
    kids.push(el("h3", {}, "params"));
    spec.params.forEach((p, i) => i > 0 && kids.push(paramRow(n, i, p)));
  } else {
    if (kindOf(n) === "out") kids.push(el("p", { class: "empty" }, "On the web every out pair plays through your speakers; channels matter for the desktop app's Logic routing."));
    spec.params.forEach((p, i) => kids.push(paramRow(n, i, p)));
  }
  kids.push(el("h3", {}, "cables"));
  const edges = state.project.edges.filter((e) => e.from === n.id || e.to === n.id);
  kids.push(el("div", { class: "links" }, edges.length ? edges.map((e) => cableRow(n, e)) : el("div", { class: "empty" }, "not connected")));
  const opening = sheet.dataset.node !== String(n.id);
  sheet.dataset.node = n.id;
  body.replaceChildren(...kids);
  body.scrollTop = opening ? 0 : keepScroll;
  requestAnimationFrame(() => {
    resize();
    if (opening) ensureVisible(n);
  });
}

// grip: drag the sheet taller / shorter
{
  let y0 = null;
  $("grip").addEventListener("pointerdown", (e) => { y0 = e.clientY; $("grip").setPointerCapture(e.pointerId); });
  $("grip").addEventListener("pointerup", (e) => {
    if (y0 === null) return;
    const dy = e.clientY - y0;
    y0 = null;
    const sheet = $("sheet");
    if (dy < -30) sheet.classList.add("tall");
    else if (dy > 60 && sheet.classList.contains("tall")) sheet.classList.remove("tall");
    else if (dy > 60) select(null);
    else sheet.classList.toggle("tall");
    requestAnimationFrame(resize);
  });
}

// ------------------------------------------------------------- overlays --

function openOverlay(...content) {
  const o = $("overlay");
  o.replaceChildren(el("div", { class: "dialog", onclick: (e) => e.stopPropagation() }, ...content));
  o.hidden = false;
}
function closeOverlay() {
  $("overlay").hidden = true;
}
$("overlay").addEventListener("click", closeOverlay);

function showAdd() {
  openOverlay(el("h2", {}, state.selected !== null ? `add node after ${label(nodeById(state.selected))}` : "add node"),
    el("div", { class: "menu kinds" }, state.specs.kinds.map((k) => el("button", { onclick: () => addNode(k.name), style: `border-left: 4px solid ${COLORS[k.name]}` },
      el("span", {}, k.name), el("small", {}, KIND_HELP[k.name])))));
}

const savedProjects = () => {
  try { return JSON.parse(localStorage.getItem(PROJECTS_KEY) || "{}"); } catch { return {}; }
};

function showMenu() {
  const item = (text, fn, cls = "") => el("button", { class: cls, onclick: () => { closeOverlay(); fn(); } }, text);
  openOverlay(el("h2", {}, "robat synth"),
    el("div", { class: "menu" },
      item("demo patch", () => confirmReplace() && loadProject(null, "demo loaded")),
      item("new (empty)", () => confirmReplace() && newProject()),
      item("save as…", saveAs),
      item("open…", showOpen),
      item("export .json", exportJson),
      item("import .json", importJson),
      item("fit view", fitView),
      item("help", showHelp),
    ));
}

function confirmReplace() {
  return state.project.nodes.length === 0 || confirm("Replace the current patch? (↶ undo brings it back)");
}

function newProject() {
  const r = edit({ op: "new" });
  if (!r.ok) return;
  setPlaying(false, { send: false });
  select(null);
  fitView();
}

function saveAs() {
  const name = prompt("save project as", "my patch");
  if (!name) return;
  const all = savedProjects();
  all[name] = state.project;
  try {
    localStorage.setItem(PROJECTS_KEY, JSON.stringify(all));
    toast(`saved “${name}”`);
  } catch {
    toast("could not save (storage full or blocked)", true);
  }
}

function showOpen() {
  const all = savedProjects();
  const names = Object.keys(all).sort();
  openOverlay(el("h2", {}, "open project"),
    names.length ? el("div", { class: "links" }, names.map((name) => el("div", { class: "link" },
      el("button", { style: "flex:1;text-align:left", onclick: () => { closeOverlay(); if (confirmReplace()) loadProject(all[name], `opened “${name}”`); } }, name),
      el("button", { class: "danger", onclick: () => { if (confirm(`delete “${name}”?`)) { delete all[name]; localStorage.setItem(PROJECTS_KEY, JSON.stringify(all)); showOpen(); } } }, "✕"))))
      : el("p", { class: "empty" }, "No saved projects yet. Use “save as…”."));
}

function exportJson() {
  const blob = new Blob([JSON.stringify(state.project, null, 2)], { type: "application/json" });
  const a = el("a", { href: URL.createObjectURL(blob), download: "robat-patch.json" });
  document.body.append(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 1000);
}

function importJson() {
  const input = el("input", { type: "file", accept: ".json,application/json" });
  input.addEventListener("change", async () => {
    const f = input.files[0];
    if (!f) return;
    try {
      loadProject(JSON.parse(await f.text()), `imported ${f.name}`);
    } catch (e) {
      toast(`not a project: ${e.message}`, true);
    }
  });
  input.click();
}

function showHelp() {
  const rows = (pairs) => el("table", {}, pairs.map(([a, b]) => el("tr", {}, el("td", {}, a), el("td", {}, b))));
  openOverlay(el("div", { class: "help" },
    el("h2", {}, "how it works"),
    el("p", {}, "Patch nodes together: seq → synth → (amp / delay) → out. Tap ▶ to play."),
    rows([
      ["tap node", "select · edit its params below"],
      ["drag node", "move it on the grid"],
      ["drag empty", "pan · pinch to zoom"],
      ["+", "add a node, auto-wired from the selection"],
      ["connect →", "then tap the target node"],
      ["loops", "audio cables may loop (even into the same node); the closing cable is dashed"],
      ["cable level", "edit a node → cables → level slider (0–2)"],
      ["↶", "undo"],
    ]),
    el("h2", { style: "margin-top:16px" }, "pattern language"),
    el("p", {}, "One cycle = one bar. C3 = MIDI 60 (Logic)."),
    rows([
      ["c3 eb4 60", "notes (octave defaults to 3) or MIDI numbers"],
      ["~", "rest"],
      ["[a b c]", "squeeze a group into one step"],
      ["<a b c>", "one per bar, in turn"],
      ["a*2  [a b]/2", "faster / slower"],
      ["a@3 b", "weight (a takes 3/4)"],
      ["a!3  a ! !", "repeat"],
      ["a, b  [c3, e3, g3]", "at the same time (chords, layers)"],
    ]),
    el("p", {}, "Example: ", el("code", {}, "c1 ~ c1 [~ c2] <eb1 g0> ~ c1 [bb0 c1]")),
    el("p", { class: "empty" }, "No sound on iPhone? Check the silent switch and volume. Patches autosave in this browser; export .json to open them in the desktop app.")));
}

let toastTimer = 0;
function toast(text, error = false) {
  const t = $("toast");
  t.textContent = text;
  t.className = error ? "error" : "";
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), error ? 3200 : 1800);
}

// ---------------------------------------------------------------- boot --

$("play").addEventListener("click", () => setPlaying(!state.playing));
$("bpm-down").addEventListener("click", () => nudgeBpm(-1));
$("bpm-up").addEventListener("click", () => nudgeBpm(1));
$("bpm").addEventListener("click", () => {
  const s = prompt("tempo (bpm)", String(state.project.bpm));
  const v = parseFloat(s);
  if (Number.isFinite(v)) { edit({ op: "bpm", bpm: v }); updateBar(); }
});
$("undo").addEventListener("click", undo);
$("menu-btn").addEventListener("click", showMenu);
$("add").addEventListener("click", showAdd);
$("fit").addEventListener("click", fitView);
window.addEventListener("resize", resize);
document.addEventListener("keydown", (e) => {
  if (e.target.tagName === "TEXTAREA" || e.target.tagName === "INPUT") return;
  if (e.key === " ") { e.preventDefault(); setPlaying(!state.playing); }
  else if (e.key === "Escape") { closeOverlay(); cancelConnect(); select(null); }
  else if ((e.key === "z" && (e.metaKey || e.ctrlKey)) || e.key === "u") undo();
});
// Resume a context the OS suspended (incoming call, tab switch) on the next touch.
document.addEventListener("pointerdown", () => {
  if (state.audio && state.playing && state.audio.ctx.state !== "running") state.audio.ctx.resume();
}, true);

async function boot() {
  const res = await fetch("robat_web.wasm");
  if (!res.ok) throw new Error(`robat_web.wasm: HTTP ${res.status}`);
  state.wasmBytes = await res.arrayBuffer();
  const { instance } = await WebAssembly.instantiate(state.wasmBytes, {});
  state.model = new Model(instance.exports);
  state.specs = state.model.call({ op: "specs" });

  let restored = null;
  try { restored = JSON.parse(localStorage.getItem(AUTOSAVE_KEY) || "null"); } catch {}
  const r = restored ? state.model.call({ op: "load", project: restored }) : null;
  setProject(r && r.ok ? r : state.model.call({ op: "demo" }));

  resize();
  fitView();
  updateBar();
  requestAnimationFrame(frame);
  // Handy from the console (and for tests): robat.state, robat.where(id).
  window.robat = { state, where: (id) => { const w = worldOf(nodeById(id)); const r = canvas.getBoundingClientRect(); const p = toScreen(w.x + NODE.w / 2, w.y + NODE.h / 2); return { x: r.left + p.x, y: r.top + p.y }; } };
  if ("serviceWorker" in navigator && location.protocol === "https:") {
    navigator.serviceWorker.register("sw.js").catch(() => {});
  }
}

boot().catch((e) => {
  console.error(e);
  document.body.append(el("pre", { style: "position:fixed;inset:auto 12px 12px;color:#ff5050;white-space:pre-wrap" }, `failed to start: ${e.message}`));
});
