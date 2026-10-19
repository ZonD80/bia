//! Cross-platform bia USB stick writer.
//!
//! Layout matches `bia-mkimage`: ESP (FAT32 BIABOOT) + BIOS boot + one ext4 BIAROOT.
//! The system comes as a minimal disk image (`image.rs`, `bia-<arch>.img.xz` in releases);
//! the writer lays its partitions over the whole stick.
//! Optional FMM keys go on the ESP as `EFI/bia/fmm.json` for the agent to seed
//! into `/home/bia/files/state/fmm.json` on first boot; the owner's language goes there
//! as `EFI/bia/lang.json`.
//! An optional GGUF from Hugging Face is streamed raw onto the end of BIAROOT and listed in
//! `EFI/bia/models.txt`; on first boot `bia-grow` moves it into `/home/bia/model` and grows
//! the ext4 over the whole stick.

mod blockio;
pub mod chatgpt;
mod check;
mod disk;
mod fmm;
pub mod hf;
pub mod lang;
pub mod image;
mod local_files;
pub mod release;
pub mod takeover;
mod write;

pub use check::{test_model, Greeting};
pub use disk::{list_removable, DiskInfo};
pub use local_files::{LocalFiles, LOCAL_FILES_NAME};
pub use fmm::{
    api_base, default_model, filter_models, fmm_config_json, is_deepseek_or_qwen_key, list_models,
    probe_key, provider_for_key, CloudModel, FmmInput, KeyGuide, ESP_FMM_REL, KEY_GUIDES, KEY_STEPS,
    LOCAL_FMM_CANDIDATES, PROVIDERS,
};
pub use image::{build_image, ImageSources};
pub use write::{parse_progress, write_stick, FlashOptions, ProgressFn, ESP_MODELS_REL, MODEL_HEADROOM};
