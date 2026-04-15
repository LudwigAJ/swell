//! File guardrails for write operations.
//!
//! This module provides validation functions for file write operations to prevent:
//! - Writing binary files (non-UTF-8 content or known binary magic bytes)
//! - Writing files exceeding size limits (default 1 MiB)
//! - Writing files to paths exceeding depth limits (default 20 levels)
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_tools::file_guardrails::{FileGuardrailConfig, validate_write_content, validate_path_depth};
//!
//! let config = FileGuardrailConfig::default();
//!
//! // Validate content before writing
//! if let Err(e) = validate_write_content(b"binary content\x00with null", &config) {
//!     println!("Binary content detected: {}", e);
//! }
//!
//! // Validate path depth
//! if let Err(e) = validate_path_depth("/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p", 20) {
//!     println!("Path too deep: {}", e);
//! }
//! ```

use std::path::Path;
use swell_core::SwellError;

/// Known binary magic bytes for common file formats.
/// These are the first few bytes that identify a file type.
const BINARY_MAGIC_BYTES: &[(&[u8], &str)] = &[
    // ELF - Unix/Linux executables
    (&[0x7F, 0x45, 0x4C, 0x46], "ELF"),
    // PNG - Image format
    (&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A], "PNG"),
    // JPEG - Image format
    (&[0xFF, 0xD8, 0xFF], "JPEG"),
    // GIF - Image format
    (&[0x47, 0x49, 0x46, 0x38, 0x37, 0x61], "GIF87a"),
    (&[0x47, 0x49, 0x46, 0x38, 0x39, 0x61], "GIF89a"),
    // PDF - Document format
    (&[0x25, 0x50, 0x44, 0x46], "PDF"),
    // ZIP - Archive format
    (&[0x50, 0x4B, 0x03, 0x04], "ZIP"),
    (&[0x50, 0x4B, 0x05, 0x06], "ZIP empty"),
    (&[0x50, 0x4B, 0x07, 0x08], "ZIP spanned"),
    // GZIP - Compressed format
    (&[0x1F, 0x8B], "GZIP"),
    // BZIP2 - Compressed format
    (&[0x42, 0x5A, 0x68], "BZIP2"),
    // RAR - Archive format
    (&[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01], "RAR"),
    // 7Z - Archive format
    (&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C], "7Z"),
    // MP3 - Audio format
    (&[0x49, 0x44, 0x33], "MP3"),
    // MP4 - Video format
    (&[0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70], "MP4"),
    (&[0x00, 0x00, 0x00, 0x1C, 0x66, 0x74, 0x79, 0x70], "MP4"),
    // WAV - Audio format
    (&[0x52, 0x49, 0x46, 0x46], "WAV"),
    // AVI - Video format
    (&[0x52, 0x49, 0x46, 0x46], "AVI"),
    // PostgreSQL dump
    (&[0x50, 0x47, 0x44, 0x42], "PostgreSQL"),
    // SQLite database
    (&[0x53, 0x51, 0x4C, 0x69, 0x74, 0x65, 0x20, 0x66], "SQLite"),
    // Java class file
    (&[0xCA, 0xFE, 0xBA, 0xBE], "Java class"),
    // Mach-O (macOS executables)
    (&[0xFE, 0xED, 0xFA, 0xCE], "Mach-O 32-bit"),
    (&[0xFE, 0xED, 0xFA, 0xCF], "Mach-O 64-bit"),
    (&[0xCE, 0xFA, 0xED, 0xFE], "Mach-O 32-bit LE"),
    (&[0xCF, 0xFA, 0xED, 0xFE], "Mach-O 64-bit LE"),
    // WebAssembly
    (&[0x00, 0x61, 0x73, 0x6D], "WebAssembly"),
    // OGG - Audio/video format
    (&[0x4F, 0x67, 0x67, 0x53], "OGG"),
    // FLAC - Audio format
    (&[0x66, 0x4C, 0x61, 0x43], "FLAC"),
    // TIFF - Image format (little endian)
    (&[0x49, 0x49, 0x2A, 0x00], "TIFF LE"),
    // TIFF - Image format (big endian)
    (&[0x4D, 0x4D, 0x00, 0x2A], "TIFF BE"),
    //ico - Icon format
    (&[0x00, 0x00, 0x01, 0x00], "ICO"),
    // PSD - Photoshop document
    (&[0x38, 0x42, 0x50, 0x53], "PSD"),
];

/// Default maximum file size: 1 MiB
pub const DEFAULT_MAX_FILE_SIZE: usize = 1_048_576;

/// Default maximum directory depth: 20 levels
pub const DEFAULT_MAX_DIRECTORY_DEPTH: usize = 20;

/// Number of bytes to check for magic bytes detection
const MAGIC_BYTES_CHECK_LEN: usize = 8192;

/// Configuration for file guardrails.
#[derive(Debug, Clone)]
pub struct FileGuardrailConfig {
    /// Maximum file size in bytes
    pub max_file_size: usize,
    /// Maximum directory depth (path components)
    pub max_directory_depth: usize,
    /// Whether to reject binary content
    pub reject_binary: bool,
    /// Whether to enforce size limits
    pub enforce_size_limit: bool,
    /// Whether to enforce depth limits
    pub enforce_depth_limit: bool,
}

impl Default for FileGuardrailConfig {
    fn default() -> Self {
        Self {
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_directory_depth: DEFAULT_MAX_DIRECTORY_DEPTH,
            reject_binary: true,
            enforce_size_limit: true,
            enforce_depth_limit: true,
        }
    }
}

impl FileGuardrailConfig {
    /// Create a new config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum file size in bytes
    pub fn with_max_file_size(mut self, size: usize) -> Self {
        self.max_file_size = size;
        self
    }

    /// Set maximum directory depth
    pub fn with_max_directory_depth(mut self, depth: usize) -> Self {
        self.max_directory_depth = depth;
        self
    }

    /// Set whether to reject binary content
    pub fn with_reject_binary(mut self, reject: bool) -> Self {
        self.reject_binary = reject;
        self
    }

    /// Set whether to enforce size limits
    pub fn with_enforce_size_limit(mut self, enforce: bool) -> Self {
        self.enforce_size_limit = enforce;
        self
    }

    /// Set whether to enforce depth limits
    pub fn with_enforce_depth_limit(mut self, enforce: bool) -> Self {
        self.enforce_depth_limit = enforce;
        self
    }
}

/// Result of binary detection analysis
#[derive(Debug, Clone)]
pub enum BinaryDetectionReason {
    /// Contains null bytes (binary indicator)
    NullBytes,
    /// Contains known binary magic bytes
    MagicBytes { format: String },
    /// Invalid UTF-8 sequence
    InvalidUtf8,
}

impl std::fmt::Display for BinaryDetectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinaryDetectionReason::NullBytes => write!(f, "content contains null bytes (binary indicator)"),
            BinaryDetectionReason::MagicBytes { format } => write!(f, "content matches {} binary format magic bytes", format),
            BinaryDetectionReason::InvalidUtf8 => write!(f, "content is not valid UTF-8"),
        }
    }
}

/// Checks if the given bytes contain null bytes, which is a strong indicator of binary content.
fn contains_null_bytes(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

/// Checks if the given bytes start with known binary magic bytes.
fn matches_binary_magic(bytes: &[u8]) -> Option<String> {
    for (magic, format_name) in BINARY_MAGIC_BYTES {
        if bytes.starts_with(magic) {
            return Some(format_name.to_string());
        }
    }
    None
}

/// Checks if the given bytes are valid UTF-8.
fn is_valid_utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
}

/// Detects if the given content is binary.
///
/// Checks in order:
/// 1. Null bytes (strong binary indicator)
/// 2. Known binary magic bytes
/// 3. UTF-8 validity (content must be valid UTF-8 to be text)
///
/// Returns `Ok(())` if content appears to be text, `Err(reason)` if binary.
pub fn detect_binary_content(content: &[u8]) -> Result<(), BinaryDetectionReason> {
    // Check first MAGIC_BYTES_CHECK_LEN bytes (or all if shorter)
    let check_len = std::cmp::min(content.len(), MAGIC_BYTES_CHECK_LEN);
    let check_bytes = &content[..check_len];

    // Check 1: Null bytes indicate binary
    if contains_null_bytes(check_bytes) {
        return Err(BinaryDetectionReason::NullBytes);
    }

    // Check 2: Known magic bytes indicate binary
    if let Some(format) = matches_binary_magic(check_bytes) {
        return Err(BinaryDetectionReason::MagicBytes { format });
    }

    // Check 3: Valid UTF-8 is required for text files
    // For content that's mostly text (like source code), we should allow
    // some invalid UTF-8 sequences that might be encoding artifacts.
    // However, if the content is clearly not UTF-8, reject it.
    // We use a heuristic: if the first 512 bytes fail UTF-8 decoding,
    // and we haven't already caught null bytes or magic bytes, consider it binary.
    let initial_check = &check_bytes[..std::cmp::min(check_bytes.len(), 512)];
    if !is_valid_utf8(initial_check) {
        // Additional check: if valid UTF-8 fails, check if this might be
        // a file with encoding issues that's still meant to be text
        // Most text files should have valid UTF-8 at the start
        return Err(BinaryDetectionReason::InvalidUtf8);
    }

    Ok(())
}

/// Validates that content intended for writing is not binary.
///
/// Returns `Ok(())` if the content is safe to write as text,
/// `Err(SwellError)` with a descriptive message if binary content is detected.
pub fn validate_write_content(content: &[u8], config: &FileGuardrailConfig) -> Result<(), SwellError> {
    if !config.reject_binary {
        return Ok(());
    }

    detect_binary_content(content).map_err(|reason| {
        SwellError::ToolExecutionFailed(format!(
            "Binary content detected in write operation: {}. \
             Binary files cannot be written via text write operations. \
             Consider using a binary-appropriate tool or encoding.",
            reason
        ))
    })
}

/// Validates that a file write operation won't exceed the configured size limit.
///
/// Returns `Ok(())` if the content size is within limits,
/// `Err(SwellError)` if the content exceeds the configured maximum.
pub fn validate_file_size(content_size: usize, config: &FileGuardrailConfig) -> Result<(), SwellError> {
    if !config.enforce_size_limit {
        return Ok(());
    }

    if content_size > config.max_file_size {
        return Err(SwellError::ToolExecutionFailed(format!(
            "File size {} bytes exceeds maximum allowed size of {} bytes ({} MiB). \
             Write operation rejected.",
            content_size,
            config.max_file_size,
            config.max_file_size / 1_048_576
        )));
    }

    Ok(())
}

/// Calculates the depth of a path (number of path components).
pub fn calculate_path_depth(path: &Path) -> usize {
    path.components().count()
}

/// Validates that a file write path doesn't exceed the configured depth limit.
///
/// Returns `Ok(())` if the path depth is within limits,
/// `Err(SwellError)` if the path exceeds the configured maximum depth.
pub fn validate_path_depth(path: &Path, config: &FileGuardrailConfig) -> Result<(), SwellError> {
    if !config.enforce_depth_limit {
        return Ok(());
    }

    let depth = calculate_path_depth(path);

    if depth > config.max_directory_depth {
        return Err(SwellError::ToolExecutionFailed(format!(
            "Path depth {} exceeds maximum allowed depth of {} levels. \
             Path '{}' is too deeply nested. Write operation rejected.",
            depth,
            config.max_directory_depth,
            path.display()
        )));
    }

    Ok(())
}

/// Validates a complete write operation: content type, size, and path depth.
///
/// This is a convenience function that combines all three validations.
/// Returns `Ok(())` if all validations pass, `Err(SwellError)` on first failure.
pub fn validate_write_operation(
    content: &[u8],
    path: &Path,
    config: &FileGuardrailConfig,
) -> Result<(), SwellError> {
    // First validate path depth (before content validation)
    validate_path_depth(path, config)?;

    // Then validate content is not binary
    validate_write_content(content, config)?;

    // Finally validate size
    validate_file_size(content.len(), config)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // =====================================================================
    // Binary Detection Tests
    // =====================================================================

    #[test]
    fn test_detect_binary_null_bytes() {
        // Null bytes indicate binary
        let binary_content = b"Hello\x00World";
        assert!(detect_binary_content(binary_content).is_err());

        let text_content = b"Hello World";
        assert!(detect_binary_content(text_content).is_ok());
    }

    #[test]
    fn test_detect_binary_elf_magic() {
        // ELF magic bytes - no null bytes in this sample
        let elf_content = b"\x7FELF";
        let result = detect_binary_content(elf_content);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            BinaryDetectionReason::MagicBytes { format } => {
                assert_eq!(format, "ELF");
            }
            _ => panic!("Expected MagicBytes error"),
        }
    }

    #[test]
    fn test_detect_binary_png_magic() {
        // PNG magic bytes - no null bytes in this sample
        let png_content = b"\x89PNG\r\n\x1A\n";
        let result = detect_binary_content(png_content);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            BinaryDetectionReason::MagicBytes { format } => {
                assert_eq!(format, "PNG");
            }
            _ => panic!("Expected MagicBytes error"),
        }
    }

    #[test]
    fn test_detect_binary_pdf_magic() {
        // PDF magic bytes
        let pdf_content = b"%PDF-1.4\n%\xb5\xb6\xb7\xb8";
        let result = detect_binary_content(pdf_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_binary_zip_magic() {
        // ZIP magic bytes
        let zip_content = b"PK\x03\x04\x00\x00\x00\x00\x00";
        let result = detect_binary_content(zip_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_binary_jpeg_magic() {
        // JPEG magic bytes
        let jpeg_content = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        let result = detect_binary_content(jpeg_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_binary_valid_utf8_text() {
        // Valid UTF-8 text should pass
        let text = "Hello, world! This is a test. 🎉";
        assert!(detect_binary_content(text.as_bytes()).is_ok());
    }

    #[test]
    fn test_detect_binary_mixed_content() {
        // Content with text followed by null bytes should be binary
        let mixed = b"Text content\x00\x00\x00binary\x00";
        assert!(detect_binary_content(mixed).is_err());
    }

    #[test]
    fn test_detect_binary_rust_source_code() {
        // Rust source code is valid text
        let rust_code = r#"
fn main() {
    println!("Hello, world!");
}
"#;
        assert!(detect_binary_content(rust_code.as_bytes()).is_ok());
    }

    #[test]
    fn test_detect_binary_markdown() {
        // Markdown is valid text
        let markdown = r#"
# Hello

This is a **test** document.

```rust
fn main() {}
```
"#;
        assert!(detect_binary_content(markdown.as_bytes()).is_ok());
    }

    // =====================================================================
    // File Size Validation Tests
    // =====================================================================

    #[test]
    fn test_validate_file_size_within_limit() {
        let config = FileGuardrailConfig::default();
        // 1 MiB - 1 byte should be OK
        let size = DEFAULT_MAX_FILE_SIZE - 1;
        assert!(validate_file_size(size, &config).is_ok());
    }

    #[test]
    fn test_validate_file_size_at_limit() {
        let config = FileGuardrailConfig::default();
        // Exactly 1 MiB should be OK
        let size = DEFAULT_MAX_FILE_SIZE;
        assert!(validate_file_size(size, &config).is_ok());
    }

    #[test]
    fn test_validate_file_size_exceeds_limit() {
        let config = FileGuardrailConfig::default();
        // 1 MiB + 1 byte should fail
        let size = DEFAULT_MAX_FILE_SIZE + 1;
        let result = validate_file_size(size, &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum allowed size"));
    }

    #[test]
    fn test_validate_file_size_custom_limit() {
        let config = FileGuardrailConfig::default().with_max_file_size(100);
        // 101 bytes should fail for 100 byte limit
        assert!(validate_file_size(100, &config).is_ok());
        assert!(validate_file_size(101, &config).is_err());
    }

    #[test]
    fn test_validate_file_size_disabled() {
        let config = FileGuardrailConfig::default().with_enforce_size_limit(false);
        // Even huge content should pass when disabled
        assert!(validate_file_size(10_000_000, &config).is_ok());
    }

    // =====================================================================
    // Directory Depth Validation Tests
    // =====================================================================

    #[test]
    fn test_calculate_path_depth() {
        // Various path formats
        assert_eq!(calculate_path_depth(Path::new("file.txt")), 1);
        assert_eq!(calculate_path_depth(Path::new("dir/file.txt")), 2);
        assert_eq!(calculate_path_depth(Path::new("a/b/c/d.txt")), 4);
        assert_eq!(calculate_path_depth(Path::new("/absolute/path/file.txt")), 4);
    }

    #[test]
    fn test_validate_path_depth_within_limit() {
        let config = FileGuardrailConfig::default();
        // 20 levels should be OK
        let path = Path::new("a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t.txt");
        assert!(validate_path_depth(path, &config).is_ok());
    }

    #[test]
    fn test_validate_path_depth_exceeds_limit() {
        let config = FileGuardrailConfig::default();
        // 21 levels should fail (default max is 20)
        let path = Path::new("a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u.txt");
        let result = validate_path_depth(path, &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum allowed depth"));
    }

    #[test]
    fn test_validate_path_depth_custom_limit() {
        let config = FileGuardrailConfig::default().with_max_directory_depth(5);
        let path = Path::new("a/b/c/d/e/f.txt"); // 6 components
        assert!(validate_path_depth(path, &config).is_err());

        let path_ok = Path::new("a/b/c/d/e.txt"); // 5 components
        assert!(validate_path_depth(path_ok, &config).is_ok());
    }

    #[test]
    fn test_validate_path_depth_disabled() {
        let config = FileGuardrailConfig::default().with_enforce_depth_limit(false);
        // Very deep paths should pass when disabled
        let path = Path::new("a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z.txt");
        assert!(validate_path_depth(path, &config).is_ok());
    }

    // =====================================================================
    // Combined Validation Tests
    // =====================================================================

    #[test]
    fn test_validate_write_operation_all_ok() {
        let config = FileGuardrailConfig::default();
        let content = b"Hello, world!";
        let path = Path::new("dir/file.txt");

        assert!(validate_write_operation(content, path, &config).is_ok());
    }

    #[test]
    fn test_validate_write_operation_binary_rejected() {
        let config = FileGuardrailConfig::default();
        let content = b"\x7FELF binary content";
        let path = Path::new("file.txt");

        let result = validate_write_operation(content, path, &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Binary content detected"));
    }

    #[test]
    fn test_validate_write_operation_size_rejected() {
        let config = FileGuardrailConfig::default().with_max_file_size(10);
        let content = b"This content is definitely longer than 10 bytes!";
        let path = Path::new("file.txt");

        let result = validate_write_operation(content, path, &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum allowed size"));
    }

    #[test]
    fn test_validate_write_operation_depth_rejected() {
        let config = FileGuardrailConfig::default().with_max_directory_depth(3);
        let content = b"content";
        let path = Path::new("a/b/c/d/e/f.txt"); // 6 components

        let result = validate_write_operation(content, path, &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum allowed depth"));
    }

    #[test]
    fn test_validate_write_operation_depth_checked_first() {
        // Depth is validated before content check
        let config = FileGuardrailConfig::default().with_max_directory_depth(3);
        let content = b"\x7FELF binary"; // Binary content
        let path = Path::new("a/b/c/d/e.txt"); // 5 components - depth fails

        let result = validate_write_operation(content, path, &config);
        assert!(result.is_err());
        // Should fail on depth, not binary detection
        assert!(result.unwrap_err().to_string().contains("exceeds maximum allowed depth"));
    }

    // =====================================================================
    // Config Builder Tests
    // =====================================================================

    #[test]
    fn test_file_guardrail_config_default() {
        let config = FileGuardrailConfig::default();

        assert_eq!(config.max_file_size, DEFAULT_MAX_FILE_SIZE);
        assert_eq!(config.max_directory_depth, DEFAULT_MAX_DIRECTORY_DEPTH);
        assert!(config.reject_binary);
        assert!(config.enforce_size_limit);
        assert!(config.enforce_depth_limit);
    }

    #[test]
    fn test_file_guardrail_config_builder() {
        let config = FileGuardrailConfig::new()
            .with_max_file_size(500_000)
            .with_max_directory_depth(10)
            .with_reject_binary(false)
            .with_enforce_size_limit(false)
            .with_enforce_depth_limit(false);

        assert_eq!(config.max_file_size, 500_000);
        assert_eq!(config.max_directory_depth, 10);
        assert!(!config.reject_binary);
        assert!(!config.enforce_size_limit);
        assert!(!config.enforce_depth_limit);
    }

    // =====================================================================
    // Edge Cases
    // =====================================================================

    #[test]
    fn test_empty_content() {
        // Empty content should pass (not binary)
        assert!(detect_binary_content(b"").is_ok());
    }

    #[test]
    fn test_single_null_byte() {
        // Single null byte should be detected as binary
        assert!(detect_binary_content(b"\x00").is_err());
    }

    #[test]
    fn test_long_content_binary_detection() {
        // For long content, we only check the first MAGIC_BYTES_CHECK_LEN bytes
        let long_binary = vec![0u8; 100_000];
        assert!(detect_binary_content(&long_binary).is_err());

        // Content with binary marker in first 8KB
        let mut content = vec![b'a'; 8192];
        content.extend_from_slice(b"\x7FELF"); // Binary marker at position 8192
        // This passes because we only check first 8192 bytes
        assert!(detect_binary_content(&content).is_ok());
    }

    #[test]
    fn test_root_path_depth() {
        // Root path should have minimal depth
        let config = FileGuardrailConfig::default();
        assert!(validate_path_depth(Path::new("/"), &config).is_ok());
        assert!(validate_path_depth(Path::new("."), &config).is_ok());
    }
}
