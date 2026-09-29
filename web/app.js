import init, { WasmPlayer } from "./pkg/libsion_wasm.js";

const PLAYER_RATE = 44100;
const BUFFER_SIZE = 4096;

// Compat shim: glue may expose camelCase renames or raw snake_case.
const isStreaming = (p) => (p.isStreaming ?? p.is_streaming).call(p);

const mmlEl = document.getElementById("mml");
const playBtn = document.getElementById("play");
const stopBtn = document.getElementById("stop");
const loopEl = document.getElementById("loop");
const volumeEl = document.getElementById("volume");
const statusEl = document.getElementById("status");
const scopeCanvas = document.getElementById("scope");
const meterFill = document.getElementById("meter-fill");
const scopeCtx = scopeCanvas.getContext("2d");

let player = null;
let audioCtx = null;
let processor = null;
let activeMml = "";
let latestPeak = 0;
let meterLevel = 0;
const scratch = new Float32Array(BUFFER_SIZE);
const scopeData = new Float32Array(BUFFER_SIZE);

function setStatus(text, state) {
  statusEl.textContent = text;
  statusEl.dataset.state = state ?? "idle";
}

function stopProcessor() {
  if (!processor) return;
  processor.onaudioprocess = null;
  processor.disconnect();
  processor = null;
}

function engineError(err) {
  stopProcessor();
  setStatus("engine error: " + (err && err.message ? err.message : String(err)), "error");
}

function startProcessor() {
  stopProcessor();
  processor = audioCtx.createScriptProcessor(BUFFER_SIZE, 1, 1);
  processor.onaudioprocess = onAudioProcess;
  processor.connect(audioCtx.destination);
}

function onAudioProcess(event) {
  const out = event.outputBuffer.getChannelData(0);
  let written = 0;
  try {
    written = Number(player.pump(scratch));
  } catch (err) {
    engineError(err);
    return;
  }
  const n = Math.min(written >>> 0, out.length);
  if (n > 0) out.set(scratch.subarray(0, n));
  if (n < out.length) out.fill(0, n);
  scopeData.set(scratch.subarray(0, n));
  if (n < scopeData.length) scopeData.fill(0, n);
  let peak = 0;
  for (let i = 0; i < n; i++) {
    const a = Math.abs(scratch[i]);
    if (a > peak) peak = a;
  }
  latestPeak = peak;
  let streaming = false;
  try {
    streaming = isStreaming(player);
  } catch (err) {
    engineError(err);
    return;
  }
  if (streaming) return;
  if (loopEl.checked) {
    try {
      player.play(activeMml);
      setStatus("playing", "playing");
    } catch (err) {
      engineError(err);
    }
  } else {
    stopProcessor();
    setStatus("finished", "finished");
  }
}

async function play() {
  if (!player) return;
  activeMml = mmlEl.value;
  try {
    if (!audioCtx) audioCtx = new AudioContext({ sampleRate: PLAYER_RATE });
    if (audioCtx.state === "suspended") await audioCtx.resume();
    player.set_volume(Number(volumeEl.value));
    if (!player.play(activeMml)) {
      setStatus("compile failed", "error");
      return;
    }
    startProcessor();
    setStatus("playing", "playing");
  } catch (err) {
    engineError(err);
  }
}

function stop() {
  try {
    if (player) player.stop();
  } finally {
    stopProcessor();
    setStatus("idle", "idle");
  }
}

function draw() {
  const w = scopeCanvas.width;
  const h = scopeCanvas.height;
  scopeCtx.fillStyle = "#0d1117";
  scopeCtx.fillRect(0, 0, w, h);
  scopeCtx.strokeStyle = "#2a3140";
  scopeCtx.beginPath();
  scopeCtx.moveTo(0, h / 2);
  scopeCtx.lineTo(w, h / 2);
  scopeCtx.stroke();
  scopeCtx.strokeStyle = "#4dd0a1";
  scopeCtx.beginPath();
  const step = Math.max(1, Math.ceil(scopeData.length / w));
  let x = 0;
  for (let i = 0; i < scopeData.length; i += step, x++) {
    const s = Math.max(-1, Math.min(1, scopeData[i]));
    const y = h / 2 - s * (h / 2 - 4);
    if (x === 0) scopeCtx.moveTo(x, y);
    else scopeCtx.lineTo(x, y);
  }
  scopeCtx.stroke();
  meterLevel = Math.max(latestPeak, meterLevel * 0.9);
  latestPeak = 0;
  meterFill.style.width = (Math.min(1, meterLevel) * 100).toFixed(1) + "%";
  requestAnimationFrame(draw);
}

async function boot() {
  try {
    await init();
    player = new WasmPlayer(PLAYER_RATE, 1);
    player.set_volume(Number(volumeEl.value));
    playBtn.disabled = false;
    stopBtn.disabled = false;
    setStatus("idle", "idle");
  } catch (err) {
    setStatus("wasm load failed: " + (err && err.message ? err.message : String(err)), "error");
    return;
  }
  requestAnimationFrame(draw);
}

volumeEl.addEventListener("input", () => {
  if (player) player.set_volume(Number(volumeEl.value));
});
playBtn.addEventListener("click", play);
stopBtn.addEventListener("click", stop);

boot();
