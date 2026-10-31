//! Streaming downloads with an inseparable hash check.
//!
//! The artifact is written to a temporary file while being hashed; only when
//! the SHA-256 matches the verified value from the release metadata is it moved
//! into place. A mismatch deletes the temporary file.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::github::sha256_hex;
use crate::net;
use crate::progress::Progress;

/// Refuse anything larger than this; the biggest Craft App tarball is < 150 MB.
pub const MAX_ARTIFACT: u64 = 512 * 1024 * 1024;

/// Downloads `url` to `dest`, verifying `expected_sha256`.
pub fn fetch(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    let response = agent
        .get(url)
        .header("Accept", "application/octet-stream")
        .call()
        .map_err(|e| Error::Http(e.to_string()))?;

    let status = response.status().as_u16();
    if status != 200 {
        return Err(Error::Http(format!(
            "Download von {url} scheiterte mit HTTP {status}"
        )));
    }

    let total = response.body().content_length();
    if total.is_some_and(|size| size > MAX_ARTIFACT) {
        return Err(Error::Http(format!(
            "File is larger than the allowed limit ({} MB)",
            MAX_ARTIFACT / 1024 / 1024
        )));
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension(format!("part-{}", std::process::id()));

    let mut body = response.into_body();
    let mut reader = body.with_config().limit(MAX_ARTIFACT).reader();
    let mut file = fs::File::create(&tmp)?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut last_report: u64 = 0;
    let mut buf = vec![0u8; 128 * 1024];

    let result = (|| -> Result<()> {
        loop {
            let read = reader.read(&mut buf)?;
            if read == 0 {
                break;
            }
            received += read as u64;
            if received > MAX_ARTIFACT {
                return Err(Error::Http("File exceeds the size limit".into()));
            }
            hasher.update(&buf[..read]);
            file.write_all(&buf[..read])?;
            if received - last_report >= 1024 * 1024 {
                last_report = received;
                progress(Progress::Download { received, total });
            }
        }
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    drop(file);

    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(&tmp);
        return Err(Error::HashMismatch {
            expected: expected_sha256.to_ascii_lowercase(),
            actual,
        });
    }

    fs::rename(&tmp, dest)?;
    progress(Progress::Download {
        received,
        total: Some(received),
    });
    Ok(())
}

/// True when `path` already has the expected checksum.
pub fn has_checksum(path: &Path, expected_sha256: &str) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    sha256_hex(&bytes).eq_ignore_ascii_case(expected_sha256)
}

/// Small bounded GET, used for icons and other auxiliary files.
pub fn fetch_bytes(agent: &ureq::Agent, url: &str, max: u64) -> Result<Vec<u8>> {
    let reply = net::get(agent, url, &[], max)?;
    if !reply.is_success() {
        return Err(Error::Http(format!(
            "{url} antwortete mit HTTP {}",
            reply.status
        )));
    }
    Ok(reply.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cached_checksums() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        fs::write(&path, b"hello").unwrap();
        assert!(has_checksum(
            &path,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        ));
        assert!(!has_checksum(&path, "00"));
    }
}
