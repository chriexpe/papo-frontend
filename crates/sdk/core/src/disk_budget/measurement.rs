//! Physical allocation measured once per encountered file; payload sizes and separate-pass subtraction are excluded.

use super::{DiskError, DiskReport};
use crate::DataRoot;
use std::{fs, path::Path};

pub(super) fn measure(root: &DataRoot, budget_bytes: u64) -> Result<DiskReport, DiskError> {
    let mut report = DiskReport {
        budget_bytes,
        ..DiskReport::default()
    };
    for name in [".lock", ".private-acl-v1"] {
        add(
            &mut report.profile_bytes,
            allocated(&root.path().join(name))?,
        )?;
    }
    report.database_bytes = database(&root.state())?;
    walk(&root.path().join("cache"), &mut |_, bytes| {
        add(&mut report.database_bytes, bytes)
    })?;
    let media = root.path().join("media");
    walk(&media, &mut |path, bytes| {
        if path.parent() == Some(media.as_path())
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| {
                    [
                        "index.sqlite",
                        "index.sqlite-wal",
                        "index.sqlite-shm",
                        "index.sqlite-journal",
                    ]
                    .contains(&name)
                })
        {
            add(&mut report.database_bytes, bytes)
        } else {
            add(&mut report.media_bytes, bytes)
        }
    })?;
    walk(&root.path().join("outgoing"), &mut |_, bytes| {
        add(&mut report.outgoing_bytes, bytes)
    })?;
    report.allocated_bytes = report
        .database_bytes
        .checked_add(report.media_bytes)
        .and_then(|n| n.checked_add(report.outgoing_bytes))
        .and_then(|n| n.checked_add(report.profile_bytes))
        .ok_or(DiskError::Measurement)?;
    report.pressure = report.allocated_bytes > budget_bytes;
    Ok(report)
}

pub(super) fn database(path: &Path) -> Result<u64, DiskError> {
    let mut bytes = 0;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        add(&mut bytes, allocated(Path::new(&name))?)?;
    }
    Ok(bytes)
}

fn add(total: &mut u64, bytes: u64) -> Result<(), DiskError> {
    *total = total.checked_add(bytes).ok_or(DiskError::Measurement)?;
    Ok(())
}

fn allocated(path: &Path) -> Result<u64, DiskError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(_) => return Err(DiskError::Measurement),
    };
    fs2::FileExt::allocated_size(&file).map_err(|_| DiskError::Measurement)
}

fn walk(
    path: &Path,
    visit: &mut impl FnMut(&Path, u64) -> Result<(), DiskError>,
) -> Result<(), DiskError> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(DiskError::Measurement),
    };
    for entry in entries {
        let entry = entry.map_err(|_| DiskError::Measurement)?;
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(DiskError::Measurement),
        };
        if kind.is_symlink() {
            return Err(DiskError::Measurement);
        }
        if kind.is_dir() {
            walk(&entry.path(), visit)?;
        } else {
            visit(&entry.path(), allocated(&entry.path())?)?;
        }
    }
    Ok(())
}
