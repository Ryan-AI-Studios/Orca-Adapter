//! ZIP open / member helpers for 3MF packages.

use std::fs::File;
use std::io::{Read, Seek, Write};

use camino::Utf8Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::{Error, Result};
use crate::paths::normalize_zip_path;

/// Open a ZIP archive from a disk path.
pub fn open_archive(path: &Utf8Path) -> Result<ZipArchive<File>> {
    let file = File::open(path.as_std_path()).map_err(|e| Error::io(path, e))?;
    ZipArchive::new(file).map_err(Error::from)
}

/// List normalized (`/`-only) member names in archive order.
pub fn list_entries<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Vec<String>> {
    let mut names = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        names.push(normalize_zip_path(file.name()));
    }
    Ok(names)
}

/// Read a ZIP member's full bytes by normalized name.
///
/// Tries exact match first; falls back to scanning normalized names if the archive
/// stored backslashes or leading slashes.
pub fn read_member_bytes<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    member: &str,
) -> Result<Vec<u8>> {
    let target = normalize_zip_path(member);
    if let Ok(mut file) = archive.by_name(&target) {
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| Error::io(format!("zip:{target}"), e))?;
        return Ok(buf);
    }
    // Fallback scan
    let index = {
        let mut found = None;
        for i in 0..archive.len() {
            let file = archive.by_index(i)?;
            if normalize_zip_path(file.name()) == target {
                found = Some(i);
                break;
            }
        }
        found
    };
    let Some(i) = index else {
        return Err(Error::MissingMember(target));
    };
    let mut file = archive.by_index(i)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| Error::io(format!("zip:{target}"), e))?;
    Ok(buf)
}

/// Create a ZipWriter writing to `path` (truncates if exists).
pub fn create_writer(path: &Utf8Path) -> Result<ZipWriter<File>> {
    if let Some(parent) = path.parent()
        && !parent.as_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent.as_std_path()).map_err(|e| Error::io(parent, e))?;
    }
    let file = File::create(path.as_std_path()).map_err(|e| Error::io(path, e))?;
    Ok(ZipWriter::new(file))
}

/// Default options for newly written (deflated) members.
pub fn deflated_options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

/// Write a new deflated member with the given bytes.
pub fn write_member<W: Write + Seek>(
    writer: &mut ZipWriter<W>,
    name: &str,
    data: &[u8],
) -> Result<()> {
    let name = normalize_zip_path(name);
    writer
        .start_file(name, deflated_options())
        .map_err(Error::from)?;
    writer
        .write_all(data)
        .map_err(|e| Error::msg(format!("failed writing ZIP member: {e}")))?;
    Ok(())
}

/// Minimal empty slice_info stub matching Wonderprint clean template style.
pub fn slice_info_stub() -> &'static [u8] {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <header>
    <header_item key="X-BBL-Client-Type" value="slicer"/>
    <header_item key="X-BBL-Client-Version" value=""/>
  </header>
</config>
"#
}
