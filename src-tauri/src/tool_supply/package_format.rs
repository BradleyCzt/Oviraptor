//! Explicit flat-bundle-v1 syntax and byte-integrity verification only. No
//! publisher/admin approval, deployment, extraction, or execution is implied.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Seek},
};

pub(crate) const MAGIC: &[u8; 8] = b"OVFB0001";
const MAX_FILES: u32 = 16;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegularFileMode {
    ReadOnlyData,
    EntryCode,
}
#[derive(Debug)]
pub(crate) struct FlatBundleEntry {
    path: String,
    content_offset: u64,
    content_bytes: u64,
    content_sha256: [u8; 32],
    mode: RegularFileMode,
}
impl FlatBundleEntry {
    pub(crate) fn path(&self) -> &str {
        &self.path
    }
    pub(crate) fn content_offset(&self) -> u64 {
        self.content_offset
    }
    pub(crate) fn content_bytes(&self) -> u64 {
        self.content_bytes
    }
    pub(crate) fn content_sha256(&self) -> &[u8; 32] {
        &self.content_sha256
    }
    pub(crate) fn mode(&self) -> RegularFileMode {
        self.mode
    }
}
/// Private construction proves syntax and hashes, never trusted origin. A
/// deployer must bind this whole digest to the actual approved package inode.
#[derive(Debug)]
pub(crate) struct VerifiedFlatBundle {
    entries: Vec<FlatBundleEntry>,
    encoded_bytes: u64,
    sha256: [u8; 32],
}
impl VerifiedFlatBundle {
    pub(crate) fn entries(&self) -> &[FlatBundleEntry] {
        &self.entries
    }
    pub(crate) fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }
    pub(crate) fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }
}

struct BoundedReader<'a, R> {
    source: &'a mut R,
    total: u64,
    hash: Sha256,
}
impl<R: Read> BoundedReader<'_, R> {
    fn exact(&mut self, buffer: &mut [u8]) -> Result<(), String> {
        let length = u64::try_from(buffer.len()).map_err(|_| "flat_bundle_size_invalid")?;
        let next = self
            .total
            .checked_add(length)
            .filter(|n| *n <= MAX_BUNDLE_BYTES)
            .ok_or("flat_bundle_size_invalid")?;
        self.source.read_exact(buffer).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                "flat_bundle_truncated"
            } else {
                "flat_bundle_read_failed"
            }
        })?;
        self.hash.update(buffer);
        self.total = next;
        Ok(())
    }
}
fn require_path(path: &str) -> Result<(), String> {
    // A portable ASCII subset of UTF-8 avoids normalization aliases across
    // filesystems. Other Unicode names are explicitly unsupported, not folded.
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || !path.is_ascii()
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
    {
        return Err("flat_bundle_path_invalid".into());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > 8 {
        return Err("flat_bundle_path_invalid".into());
    }
    for part in parts {
        if part.is_empty() || matches!(part, "." | "..") || part.ends_with('.') {
            return Err("flat_bundle_path_invalid".into());
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err("flat_bundle_path_invalid".into());
        }
    }
    Ok(())
}

pub(crate) fn parse_flat_bundle_v1<R: Read + Seek>(
    source: &mut R,
    entrypoint: &str,
) -> Result<VerifiedFlatBundle, String> {
    require_path(entrypoint)?;
    if source
        .stream_position()
        .map_err(|_| "flat_bundle_seek_failed")?
        != 0
    {
        return Err("flat_bundle_source_position_invalid".into());
    }
    let mut reader = BoundedReader {
        source,
        total: 0,
        hash: Sha256::new(),
    };
    let mut header = [0u8; 14];
    reader.exact(&mut header)?;
    if &header[..8] != MAGIC {
        return Err("flat_bundle_magic_invalid".into());
    }
    if u16::from_le_bytes([header[8], header[9]]) != 1 {
        return Err("flat_bundle_version_unsupported".into());
    }
    let count = u32::from_le_bytes([header[10], header[11], header[12], header[13]]);
    if count == 0 || count > MAX_FILES {
        return Err("flat_bundle_file_count_invalid".into());
    }
    let mut entries = Vec::new();
    let mut paths = BTreeSet::<String>::new();
    let mut entry_count = 0;
    for _ in 0..count {
        let mut record = [0u8; 43];
        reader.exact(&mut record)?;
        let path_size = usize::from(u16::from_le_bytes([record[0], record[1]]));
        if path_size == 0 || path_size > MAX_PATH_BYTES {
            return Err("flat_bundle_path_invalid".into());
        }
        let content_bytes = u64::from_le_bytes(
            record[2..10]
                .try_into()
                .map_err(|_| "flat_bundle_size_invalid")?,
        );
        if content_bytes > MAX_FILE_BYTES {
            return Err("flat_bundle_file_size_invalid".into());
        }
        let mode = match record[10] {
            0 => RegularFileMode::ReadOnlyData,
            1 => RegularFileMode::EntryCode,
            _ => return Err("flat_bundle_file_mode_invalid".into()),
        };
        let content_sha256: [u8; 32] = record[11..43]
            .try_into()
            .map_err(|_| "flat_bundle_digest_invalid")?;
        let mut path_bytes = vec![0; path_size];
        reader.exact(&mut path_bytes)?;
        let path = String::from_utf8(path_bytes).map_err(|_| "flat_bundle_path_invalid")?;
        require_path(&path)?;
        let normalized = path.to_ascii_lowercase();
        if paths.iter().any(|old| {
            old == &normalized
                || old.starts_with(&(normalized.clone() + "/"))
                || normalized.starts_with(&(old.clone() + "/"))
        }) {
            return Err("flat_bundle_path_conflict".into());
        }
        paths.insert(normalized);
        if mode == RegularFileMode::EntryCode {
            entry_count += 1;
            if path != entrypoint || content_bytes == 0 {
                return Err("flat_bundle_entrypoint_invalid".into());
            }
        } else if path == entrypoint {
            return Err("flat_bundle_entrypoint_invalid".into());
        }
        reader
            .total
            .checked_add(content_bytes)
            .filter(|n| *n <= MAX_BUNDLE_BYTES)
            .ok_or("flat_bundle_size_invalid")?;
        let content_offset = reader.total;
        let mut remaining = content_bytes;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let size = usize::try_from(
                remaining.min(u64::try_from(buffer.len()).map_err(|_| "flat_bundle_size_invalid")?),
            )
            .map_err(|_| "flat_bundle_size_invalid")?;
            reader.exact(&mut buffer[..size])?;
            hash.update(&buffer[..size]);
            remaining -= u64::try_from(size).map_err(|_| "flat_bundle_size_invalid")?;
        }
        let actual: [u8; 32] = hash.finalize().into();
        if actual != content_sha256 {
            return Err("flat_bundle_digest_invalid".into());
        }
        entries.push(FlatBundleEntry {
            path,
            content_offset,
            content_bytes,
            content_sha256,
            mode,
        });
    }
    if entry_count != 1 {
        return Err("flat_bundle_entrypoint_invalid".into());
    }
    let mut tail = [0u8; 1];
    match reader.source.read_exact(&mut tail) {
        Ok(()) => return Err("flat_bundle_trailing_bytes".into()),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {}
        Err(_) => return Err("flat_bundle_read_failed".into()),
    }
    if reader
        .source
        .stream_position()
        .map_err(|_| "flat_bundle_seek_failed")?
        != reader.total
    {
        return Err("flat_bundle_source_position_invalid".into());
    }
    Ok(VerifiedFlatBundle {
        entries,
        encoded_bytes: reader.total,
        sha256: reader.hash.finalize().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(entries: &[(&[u8], u8, &[u8])]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(u32::try_from(entries.len()).unwrap().to_le_bytes());
        for (path, mode, content) in entries {
            bytes.extend(u16::try_from(path.len()).unwrap().to_le_bytes());
            bytes.extend(u64::try_from(content.len()).unwrap().to_le_bytes());
            bytes.push(*mode);
            bytes.extend(Sha256::digest(content));
            bytes.extend(*path);
            bytes.extend(*content);
        }
        bytes
    }
    fn parse(bytes: &[u8]) -> Result<VerifiedFlatBundle, String> {
        parse_flat_bundle_v1(&mut std::io::Cursor::new(bytes), "bin/parser")
    }
    #[test]
    fn flat_bundle_real_bytes_integrity_and_offsets_are_bounded() {
        let bytes = fixture(&[
            (b"bin/parser", 1, b"fixed-parser"),
            (b"data/input.json", 0, b"{}"),
        ]);
        let verified = parse(&bytes).unwrap();
        assert_eq!(
            verified.encoded_bytes(),
            u64::try_from(bytes.len()).unwrap()
        );
        assert_eq!(*verified.sha256(), <[u8; 32]>::from(Sha256::digest(&bytes)));
        for entry in verified.entries() {
            let start = usize::try_from(entry.content_offset()).unwrap();
            let end = start + usize::try_from(entry.content_bytes()).unwrap();
            assert_eq!(
                *entry.content_sha256(),
                <[u8; 32]>::from(Sha256::digest(&bytes[start..end]))
            );
        }
        assert_eq!(verified.entries()[0].path(), "bin/parser");
        assert_eq!(verified.entries()[0].mode(), RegularFileMode::EntryCode);
        assert_eq!(verified.entries()[1].mode(), RegularFileMode::ReadOnlyData);
    }
    #[test]
    fn flat_bundle_truncation_lengths_version_digest_and_tail_are_rejected() {
        let original = fixture(&[(b"bin/parser", 1, b"fixed-parser")]);
        for end in 0..original.len() {
            assert!(parse(&original[..end]).is_err(), "truncated at {end}");
        }
        let mut changed = original.clone();
        changed[0] ^= 1;
        assert_eq!(parse(&changed).unwrap_err(), "flat_bundle_magic_invalid");
        let mut changed = original.clone();
        changed[8] = 2;
        assert_eq!(
            parse(&changed).unwrap_err(),
            "flat_bundle_version_unsupported"
        );
        let mut changed = original.clone();
        changed[10..14].copy_from_slice(&17u32.to_le_bytes());
        assert_eq!(
            parse(&changed).unwrap_err(),
            "flat_bundle_file_count_invalid"
        );
        let mut changed = original.clone();
        changed[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(
            parse(&changed).unwrap_err(),
            "flat_bundle_file_size_invalid"
        );
        let mut changed = original.clone();
        changed[25] ^= 1;
        assert_eq!(parse(&changed).unwrap_err(), "flat_bundle_digest_invalid");
        let mut changed = original;
        changed.push(0);
        assert_eq!(parse(&changed).unwrap_err(), "flat_bundle_trailing_bytes");
    }
    #[test]
    fn flat_bundle_paths_links_duplicates_and_missing_entrypoint_are_rejected() {
        for bad in [
            b"../outside".as_slice(),
            b"/absolute",
            b"a\\b",
            b"a//b",
            b"a/./b",
            b"C:escape",
            b"NUL.txt",
            b"bad.",
            b"a\0b",
            b"\xff",
            "资料".as_bytes(),
        ] {
            assert_eq!(
                parse(&fixture(&[(b"bin/parser", 1, b"x"), (bad, 0, b"y")])).unwrap_err(),
                "flat_bundle_path_invalid"
            );
        }
        for path in [
            b"bin/parser".as_slice(),
            b"BIN/PARSER",
            b"bin",
            b"bin/parser/child",
        ] {
            assert_eq!(
                parse(&fixture(&[(b"bin/parser", 1, b"x"), (path, 0, b"y")])).unwrap_err(),
                "flat_bundle_path_conflict"
            );
        }
        for mode in [2, 3, 255] {
            assert_eq!(
                parse(&fixture(&[(b"bin/parser", mode, b"x")])).unwrap_err(),
                "flat_bundle_file_mode_invalid"
            );
        }
        assert_eq!(
            parse(&fixture(&[(b"data/only", 0, b"x")])).unwrap_err(),
            "flat_bundle_entrypoint_invalid"
        );
        assert_eq!(
            parse(&fixture(&[(b"bin/parser", 1, b"")])).unwrap_err(),
            "flat_bundle_entrypoint_invalid"
        );
    }
    #[test]
    fn flat_bundle_io_failures_and_oversize_declarations_do_not_read_payload() {
        struct Faulty {
            cursor: std::io::Cursor<Vec<u8>>,
            read_fail: bool,
            seek_fail: bool,
            read_bytes: usize,
        }
        impl Read for Faulty {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.read_fail {
                    return Err(std::io::Error::other("owned-read-fault"));
                }
                let n = self.cursor.read(buf)?;
                self.read_bytes += n;
                Ok(n)
            }
        }
        impl Seek for Faulty {
            fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
                if self.seek_fail {
                    return Err(std::io::Error::other("owned-seek-fault"));
                }
                self.cursor.seek(pos)
            }
        }
        let bytes = fixture(&[(b"bin/parser", 1, b"x")]);
        for (read_fail, seek_fail, expected) in [
            (true, false, "flat_bundle_read_failed"),
            (false, true, "flat_bundle_seek_failed"),
        ] {
            let mut source = Faulty {
                cursor: std::io::Cursor::new(bytes.clone()),
                read_fail,
                seek_fail,
                read_bytes: 0,
            };
            assert_eq!(
                parse_flat_bundle_v1(&mut source, "bin/parser").unwrap_err(),
                expected
            );
            assert_eq!(source.read_bytes, 0);
        }
        let mut large = bytes.clone();
        large[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        let mut source = Faulty {
            cursor: std::io::Cursor::new(large),
            read_fail: false,
            seek_fail: false,
            read_bytes: 0,
        };
        assert_eq!(
            parse_flat_bundle_v1(&mut source, "bin/parser").unwrap_err(),
            "flat_bundle_file_size_invalid"
        );
        assert_eq!(
            source.read_bytes,
            14 + 43,
            "size rejection precedes path or content reading/allocation"
        );
        let mut cursor = std::io::Cursor::new(bytes);
        cursor.set_position(1);
        assert_eq!(
            parse_flat_bundle_v1(&mut cursor, "bin/parser").unwrap_err(),
            "flat_bundle_source_position_invalid"
        );
    }
}
