pub const TEMP_PATH: &str = "tmp";
pub const INPUT_PATH: &str = "in";
pub const OUTPUT_PATH: &str = "out";

#[cfg(target_os = "linux")]
pub const UPX_PATH: &str = "bin/upx/upx";

#[cfg(target_os = "windows")]
pub const UPX_PATH: &str = "bin/upx/upx.exe";
