use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// An expected file digest, written `sha256:<hex>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checksum {
    expected: String,
}

impl FromStr for Checksum {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let (algo, hex) = s.split_once([':', '=']).ok_or_else(|| Error::BadChecksum(s.into()))?;
        let algo = algo.to_ascii_lowercase().replace('-', "");
        let hex = hex.trim().to_ascii_lowercase();
        if algo != "sha256" || hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::BadChecksum(s.into()));
        }
        Ok(Self { expected: hex })
    }
}

impl Checksum {
    pub fn verify_file(&self, path: &Path) -> Result<()> {
        let actual = sha256_file(path)?;
        if actual == self.expected {
            Ok(())
        } else {
            Err(Error::ChecksumMismatch { expected: self.expected.clone(), actual })
        }
    }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_spellings() {
        let hex = "a".repeat(64);
        assert!(format!("sha256:{hex}").parse::<Checksum>().is_ok());
        assert!(format!("sha-256={hex}").parse::<Checksum>().is_ok());
        assert!(format!("md5:{hex}").parse::<Checksum>().is_err());
        assert!("sha256:abc".parse::<Checksum>().is_err());
    }
}
