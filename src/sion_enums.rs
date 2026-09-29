//! Port of `src/sion_enums.h`. The C++ enums are used as plain integers with
//! arithmetic (e.g. `PULSE_PULSE+[0,15]`), so they are ported as constants.

pub const CHIP_AUTO: i32 = -1;
pub const CHIP_SIOPM: i32 = 0;
pub const CHIP_OPL: i32 = 1;
pub const CHIP_OPM: i32 = 2;
pub const CHIP_OPN: i32 = 3;
pub const CHIP_OPX: i32 = 4;
pub const CHIP_MA3: i32 = 5;
pub const CHIP_PMS_GUITAR: i32 = 6;
pub const CHIP_ANALOG_LIKE: i32 = 7;
pub const CHIP_MAX: i32 = 8;

pub const MODULE_PSG: i32 = 0; // PSG (DCSG)
pub const MODULE_APU: i32 = 1; // FC pAPU
pub const MODULE_NOISE: i32 = 2; // Noise wave
pub const MODULE_MA3: i32 = 3; // MA-3 wave form
pub const MODULE_SCC: i32 = 4; // SCC-like wave table
pub const MODULE_GENERIC_PG: i32 = 5; // Generic pulse generator
pub const MODULE_FM: i32 = 6; // FM sound module
pub const MODULE_PCM: i32 = 7; // PCM
pub const MODULE_PULSE: i32 = 8; // Pulse wave
pub const MODULE_RAMP: i32 = 9; // Ramp wave
pub const MODULE_SAMPLE: i32 = 10; // Sampler
pub const MODULE_KS: i32 = 11; // Karplus-Strong
pub const MODULE_GB: i32 = 12; // GameBoy-like
pub const MODULE_VRC6: i32 = 13; // VRC6
pub const MODULE_SID: i32 = 14; // SID
pub const MODULE_FM_OPM: i32 = 15; // YM2151
pub const MODULE_FM_OPN: i32 = 16; // YM2203
pub const MODULE_FM_OPNA: i32 = 17; // YM2608
pub const MODULE_FM_OPLL: i32 = 18; // YM2413
pub const MODULE_FM_OPL3: i32 = 19; // YM3812
pub const MODULE_FM_MA3: i32 = 20; // YMU762
pub const MODULE_MAX: i32 = 21;

pub const PITCH_TABLE_OPM: i32 = 0;
pub const PITCH_TABLE_PCM: i32 = 1;
pub const PITCH_TABLE_PSG: i32 = 2;
pub const PITCH_TABLE_OPM_NOISE: i32 = 3;
pub const PITCH_TABLE_PSG_NOISE: i32 = 4;
pub const PITCH_TABLE_APU_NOISE: i32 = 5;
pub const PITCH_TABLE_GB_NOISE: i32 = 6;
pub const PITCH_TABLE_MAX: usize = 7;

// A.k.a. wave/waveform shapes.
pub const PULSE_SINE: i32 = 0; // sine wave.
pub const PULSE_SAW_UP: i32 = 1; // upwards saw wave.
pub const PULSE_SAW_DOWN: i32 = 2; // downwards saw wave.
pub const PULSE_TRIANGLE_FC: i32 = 3; // triangle wave quantized by 4 bits.
pub const PULSE_TRIANGLE: i32 = 4; // triangle wave.
pub const PULSE_SQUARE: i32 = 5; // square wave.
pub const PULSE_NOISE: i32 = 6; // 32k white noise.
pub const PULSE_KNM_BUBBLE: i32 = 7; // Konami bubble system wave.
pub const PULSE_SYNC_LOW: i32 = 8; // pseudo sync (low frequency).
pub const PULSE_SYNC_HIGH: i32 = 9; // pseudo sync (high frequency).
pub const PULSE_OFFSET: i32 = 10; // reserved value, unused.
pub const PULSE_SAW_VC6: i32 = 11; // VC6 saw (32-sample saw).

pub const PULSE_NOISE_WHITE: i32 = 16; // 16k white noise.
pub const PULSE_NOISE_PULSE: i32 = 17; // 16k pulse noise.
pub const PULSE_NOISE_SHORT: i32 = 18; // fc short noise.
pub const PULSE_NOISE_HIPASS: i32 = 19; // high pass noise.
pub const PULSE_NOISE_PINK: i32 = 20; // pink noise.
pub const PULSE_NOISE_GB_SHORT: i32 = 21; // GameBoy-like short noise.

pub const PULSE_PC_NZ_16BIT: i32 = 24; // pitch controllable periodic noise
pub const PULSE_PC_NZ_SHORT: i32 = 25; // pitch controllable 93-byte noise
pub const PULSE_PC_NZ_OPM: i32 = 26; // pulse noise with OPM noise table

pub const PULSE_MA3_SINE: i32 = 32;
pub const PULSE_MA3_SINE_HALF: i32 = 33;
pub const PULSE_MA3_SINE_HALF_DOUBLE: i32 = 34;
pub const PULSE_MA3_SINE_QUART_DOUBLE: i32 = 35;
pub const PULSE_MA3_SINE_X2: i32 = 36;
pub const PULSE_MA3_SINE_HALF_DOUBLE_X2: i32 = 37;
pub const PULSE_MA3_SQUARE: i32 = 38;
pub const PULSE_MA3_SAW_SINE: i32 = 39;
pub const PULSE_MA3_TRI_SINE: i32 = 40;
pub const PULSE_MA3_TRI_SINE_HALF: i32 = 41;
pub const PULSE_MA3_TRI_SINE_HALF_DOUBLE: i32 = 42;
pub const PULSE_MA3_TRI_SINE_QUART_DOUBLE: i32 = 43;
pub const PULSE_MA3_TRI_SINE_X2: i32 = 44;
pub const PULSE_MA3_TRI_SINE_HALF_DOUBLE_X2: i32 = 45;
pub const PULSE_MA3_SQUARE_HALF: i32 = 46;
pub const PULSE_MA3_USER1: i32 = 47;
pub const PULSE_MA3_TRI: i32 = 48;
pub const PULSE_MA3_TRI_HALF: i32 = 49;
pub const PULSE_MA3_TRI_HALF_DOUBLE: i32 = 50;
pub const PULSE_MA3_TRI_QUART_DOUBLE: i32 = 51;
pub const PULSE_MA3_TRI_X2: i32 = 52;
pub const PULSE_MA3_TRI_HALF_DOUBLE_X2: i32 = 53;
pub const PULSE_MA3_SQUARE_QUART_DOUBLE: i32 = 54;
pub const PULSE_MA3_USER2: i32 = 55;
pub const PULSE_MA3_SAW: i32 = 56;
pub const PULSE_MA3_SAW_HALF: i32 = 57;
pub const PULSE_MA3_SAW_HALF_DOUBLE: i32 = 58;
pub const PULSE_MA3_SAW_QUART_DOUBLE: i32 = 59;
pub const PULSE_MA3_SAW_X2: i32 = 60;
pub const PULSE_MA3_SAW_HALF_DOUBLE_X2: i32 = 61;
pub const PULSE_MA3_SQUARE_QUART: i32 = 62;
pub const PULSE_MA3_USER3: i32 = 63;

pub const PULSE_PULSE: i32 = 64; // (64-79) square pulse wave. PULSE_PULSE+[0,15]
pub const PULSE_PULSE_SPIKE: i32 = 80; // (80-95) square pulse wave. PULSE_PULSE_SPIKE+[0,15]

pub const PULSE_RAMP: i32 = 128; // (128-255) ramp waves. PULSE_RAMP+[0,127]

pub const PULSE_CUSTOM: i32 = 256; // (256-383) custom wave table. PULSE_CUSTOM+[0,127]
pub const PULSE_PCM: i32 = 384; // (384-511) PCM data. PULSE_PCM+[0,127]

pub const PULSE_USER_CUSTOM: i32 = -1; // User registered custom wave table.
pub const PULSE_USER_PCM: i32 = -2; // User registered PCM data.
