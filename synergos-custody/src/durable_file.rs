// @implements SPEC-SY-custody-durability
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;

/// A cancelled request removes its partial file. Crash leftovers are reaped at startup.
pub struct PartialFile(pub PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(%error,"custody partial cleanup failed");
            }
        }
    }
}
pub async fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    let partial = PartialFile(parent.join(format!("{}.part", uuid::Uuid::new_v4())));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial.0)
        .await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    publish(&partial.0, path).await
}
pub async fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    let source = source.to_owned();
    let destination = destination.to_owned();
    tokio::task::spawn_blocking(move || publish_sync(&source, &destination))
        .await
        .map_err(io::Error::other)?
}
#[cfg(unix)]
fn publish_sync(source: &Path, destination: &Path) -> io::Result<()> {
    // Same-directory hard link gives create-if-absent, followed by directory fsync.
    std::fs::hard_link(source, destination)?;
    std::fs::File::open(
        destination
            .parent()
            .ok_or_else(|| io::Error::other("missing parent"))?,
    )?
    .sync_all()?;
    Ok(())
}
#[cfg(windows)]
fn publish_sync(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // No REPLACE_EXISTING flag: a published object is immutable.
    // https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
#[cfg(not(any(unix, windows)))]
fn publish_sync(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "durable publication unsupported on this platform",
    ))
}
