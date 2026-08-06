//! Crate-wide error types — every subsystem error lives here.
//!
//! Pipeline/CLI code works with `anyhow::Error` for ergonomic `.context()`
//! chaining. Typed errors (`?` + `#[from]`) flow through the umbrella
//! [`Error`] enum automatically.

use thiserror::Error;

// ---------------------------------------------------------------------------
// Crypto — Fernus KK / Blowfish / KrySWFCrypto
// ---------------------------------------------------------------------------

/// Errors raised by the Fernus string/SWF crypto layer (KK, KrySWFCrypto,
/// KryCode parsing).
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("{context}: {source}")]
    Base64 {
        context: &'static str,
        #[source]
        source: base64::DecodeError,
    },

    #[error("{context}: not valid UTF-8")]
    Utf8 {
        context: &'static str,
        #[source]
        source: std::string::FromUtf8Error,
    },

    #[error("invalid Blowfish key: {0}")]
    InvalidKey(&'static str),

    #[error("ciphertext length {0} not a multiple of 8")]
    BadCiphertextLen(usize),

    #[error("Blowfish key init failed")]
    KeyInit(#[from] blowfish::cipher::InvalidLength),

    #[error("Blowfish block slice was not 8 bytes")]
    BadBlockSlice(#[from] std::array::TryFromSliceError),

    #[error("invalid PKCS5/PKCS7 padding: length {0}")]
    InvalidPadding(usize),

    #[error("invalid fernusCode: {0}")]
    InvalidFernusCode(String),
}

// ---------------------------------------------------------------------------
// Frns — Fernus .frns v2 format
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum FrnsError {
    #[error("LZMA data too short: {got} bytes (need ≥ {min})")]
    LzmaTooShort { got: usize, min: usize },

    #[error("LZMA {context}: {source}")]
    Lzma {
        context: &'static str,
        #[source]
        source: lzma_rs::error::Error,
    },

    #[error("frame {frame}: {message}")]
    Frame { frame: u32, message: &'static str },

    #[error("{context} base64 decode")]
    Base64 {
        context: &'static str,
        #[source]
        source: base64::DecodeError,
    },

    #[error("xor-decoded data not valid UTF-8")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

// ---------------------------------------------------------------------------
// V3 — v3 Fernus (Flutter + Enigma) / AES-CBC
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum V3Error {
    #[error("envied array mismatch: tag={tag} expected={expected}")]
    ArrayMismatch { tag: char, expected: &'static str },

    #[error("envied array {0} not found")]
    ArrayNotFound(String),

    #[error("envied array not terminated")]
    ArrayUnterminated,

    #[error("envied array int parse failed: {0}")]
    BadInt(std::num::ParseIntError),

    #[error("regex compile failed")]
    RegexCompile(#[from] regex::Error),

    #[error("iv array not found")]
    IvNotFound,

    #[error("serialize section not valid UTF-8")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("AES key must be 32 bytes, got {0}")]
    AesKeyLen(usize),

    #[error("AES IV must be 16 bytes, got {0}")]
    AesIvLen(usize),

    #[error("AES ciphertext length {0} not a multiple of 16")]
    AesBlockLen(usize),

    #[error("invalid PKCS7 padding")]
    InvalidPkcs7,
}

// ---------------------------------------------------------------------------
// Swf — SWF normalisation / parse / patch
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SwfError {
    #[error("SWF too short: {0} bytes")]
    TooShort(usize),

    #[error("zlib inflate failed: {0}")]
    Zlib(#[source] std::io::Error),

    #[error("ZWS (LZMA) SWF is not supported")]
    UnsupportedZws,

    #[error("not a SWF signature: {0:?}")]
    BadSignature(String),

    #[error("failed to decompress SWF: {0}")]
    Decompress(String),

    #[error("swf parse failed: {0}")]
    Parse(String),

    #[error("swf re-encode failed: {0}")]
    Reencode(String),

    #[error("invalid patch {which}: {dim} (must be positive finite)")]
    BadPatchDim { dim: f64, which: &'static str },
}

// ---------------------------------------------------------------------------
// Umbrella
// ---------------------------------------------------------------------------

/// Top-level error returned by typed APIs across zKitap2Pdf.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Crypto(#[from] CryptoError),

    #[error(transparent)]
    Frns(#[from] FrnsError),

    #[error(transparent)]
    V3(#[from] V3Error),

    #[error(transparent)]
    Swf(#[from] SwfError),

    #[error(transparent)]
    Enigma(#[from] evbunpack_rs::error::EnigmaError),

    #[error(transparent)]
    Aplib(#[from] evbunpack_rs::error::AplibError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

// Convenience alias so internal APIs can use `crate::error::Result<T>`
// without spelling out the Error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
