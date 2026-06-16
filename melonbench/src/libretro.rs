//! Minimal libretro API surface needed to host a core headlessly.
//! Constants and structs transcribed from libretro.h (API v1).

#![allow(dead_code, non_camel_case_types)]

use std::ffi::c_void;
use std::os::raw::c_char;

pub const RETRO_API_VERSION: u32 = 1;

pub const RETRO_ENVIRONMENT_EXPERIMENTAL: u32 = 0x10000;

// Environment command numbers (experimental flag stripped before matching).
pub const ENV_SET_ROTATION: u32 = 1;
pub const ENV_GET_OVERSCAN: u32 = 2;
pub const ENV_GET_CAN_DUPE: u32 = 3;
pub const ENV_SET_MESSAGE: u32 = 6;
pub const ENV_SHUTDOWN: u32 = 7;
pub const ENV_GET_SYSTEM_DIRECTORY: u32 = 9;
pub const ENV_SET_PIXEL_FORMAT: u32 = 10;
pub const ENV_SET_INPUT_DESCRIPTORS: u32 = 11;
pub const ENV_SET_HW_RENDER: u32 = 14;
pub const ENV_GET_VARIABLE: u32 = 15;
pub const ENV_SET_VARIABLES: u32 = 16;
pub const ENV_GET_VARIABLE_UPDATE: u32 = 17;
pub const ENV_SET_SUPPORT_NO_GAME: u32 = 18;
pub const ENV_GET_LOG_INTERFACE: u32 = 27;
pub const ENV_GET_PERF_INTERFACE: u32 = 28;
pub const ENV_GET_SAVE_DIRECTORY: u32 = 31;
pub const ENV_SET_SYSTEM_AV_INFO: u32 = 32;
pub const ENV_SET_CONTROLLER_INFO: u32 = 35;
pub const ENV_SET_GEOMETRY: u32 = 37;
pub const ENV_GET_LANGUAGE: u32 = 39;
pub const ENV_SET_SUPPORT_ACHIEVEMENTS: u32 = 42;
pub const ENV_GET_VFS_INTERFACE: u32 = 45;
pub const ENV_GET_AUDIO_VIDEO_ENABLE: u32 = 47;
pub const ENV_GET_INPUT_BITMASKS: u32 = 51;
pub const ENV_GET_CORE_OPTIONS_VERSION: u32 = 52;
pub const ENV_SET_CORE_OPTIONS: u32 = 53;
pub const ENV_SET_CORE_OPTIONS_INTL: u32 = 54;
pub const ENV_SET_CORE_OPTIONS_DISPLAY: u32 = 55;
pub const ENV_GET_MESSAGE_INTERFACE_VERSION: u32 = 59;
pub const ENV_SET_MESSAGE_EXT: u32 = 60;
pub const ENV_SET_CONTENT_INFO_OVERRIDE: u32 = 65;
pub const ENV_SET_CORE_OPTIONS_V2: u32 = 67;
pub const ENV_SET_CORE_OPTIONS_V2_INTL: u32 = 68;
pub const ENV_SET_CORE_OPTIONS_UPDATE_DISPLAY_CALLBACK: u32 = 69;
pub const ENV_GET_JIT_CAPABLE: u32 = 74;

pub const RETRO_PIXEL_FORMAT_0RGB1555: u32 = 0;
pub const RETRO_PIXEL_FORMAT_XRGB8888: u32 = 1;
pub const RETRO_PIXEL_FORMAT_RGB565: u32 = 2;

pub const RETRO_DEVICE_JOYPAD: u32 = 1;

// Joypad button ids, used by the --input schedule parser.
pub const JOYPAD_BUTTONS: &[(&str, u32)] = &[
    ("B", 0),
    ("Y", 1),
    ("SELECT", 2),
    ("START", 3),
    ("UP", 4),
    ("DOWN", 5),
    ("LEFT", 6),
    ("RIGHT", 7),
    ("A", 8),
    ("X", 9),
    ("L", 10),
    ("R", 11),
];

#[repr(C)]
pub struct retro_variable {
    pub key: *const c_char,
    pub value: *const c_char,
}

#[repr(C)]
pub struct retro_message {
    pub msg: *const c_char,
    pub frames: u32,
}

#[repr(C)]
pub struct retro_game_info {
    pub path: *const c_char,
    pub data: *const c_void,
    pub size: usize,
    pub meta: *const c_char,
}

#[repr(C)]
pub struct retro_game_geometry {
    pub base_width: u32,
    pub base_height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub aspect_ratio: f32,
}

#[repr(C)]
pub struct retro_system_timing {
    pub fps: f64,
    pub sample_rate: f64,
}

#[repr(C)]
pub struct retro_system_av_info {
    pub geometry: retro_game_geometry,
    pub timing: retro_system_timing,
}

#[repr(C)]
pub struct retro_system_info {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub need_fullpath: bool,
    pub block_extract: bool,
}

// Callback signatures the core expects from the frontend.
pub type retro_environment_t = unsafe extern "C" fn(cmd: u32, data: *mut c_void) -> bool;
pub type retro_video_refresh_t =
    unsafe extern "C" fn(data: *const c_void, width: u32, height: u32, pitch: usize);
pub type retro_audio_sample_t = unsafe extern "C" fn(left: i16, right: i16);
pub type retro_audio_sample_batch_t =
    unsafe extern "C" fn(data: *const i16, frames: usize) -> usize;
pub type retro_input_poll_t = unsafe extern "C" fn();
pub type retro_input_state_t =
    unsafe extern "C" fn(port: u32, device: u32, index: u32, id: u32) -> i16;

// Core entry points resolved via GetProcAddress.
pub type retro_set_environment_t = unsafe extern "C" fn(retro_environment_t);
pub type retro_set_video_refresh_t = unsafe extern "C" fn(retro_video_refresh_t);
pub type retro_set_audio_sample_t = unsafe extern "C" fn(retro_audio_sample_t);
pub type retro_set_audio_sample_batch_t = unsafe extern "C" fn(retro_audio_sample_batch_t);
pub type retro_set_input_poll_t = unsafe extern "C" fn(retro_input_poll_t);
pub type retro_set_input_state_t = unsafe extern "C" fn(retro_input_state_t);
pub type retro_init_t = unsafe extern "C" fn();
pub type retro_deinit_t = unsafe extern "C" fn();
pub type retro_api_version_t = unsafe extern "C" fn() -> u32;
pub type retro_get_system_info_t = unsafe extern "C" fn(*mut retro_system_info);
pub type retro_get_system_av_info_t = unsafe extern "C" fn(*mut retro_system_av_info);
pub type retro_load_game_t = unsafe extern "C" fn(*const retro_game_info) -> bool;
pub type retro_unload_game_t = unsafe extern "C" fn();
pub type retro_run_t = unsafe extern "C" fn();

// Memory access — exactly what RetroAchievements reads to evaluate achievements.
pub const RETRO_MEMORY_SAVE_RAM: u32 = 0;
pub const RETRO_MEMORY_SYSTEM_RAM: u32 = 2;
pub type retro_get_memory_data_t = unsafe extern "C" fn(id: u32) -> *mut c_void;
pub type retro_get_memory_size_t = unsafe extern "C" fn(id: u32) -> usize;
