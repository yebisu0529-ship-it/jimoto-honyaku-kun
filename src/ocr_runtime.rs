//! Cancellation and process ownership shared by optional local OCR backends.
use anyhow::{Context, Result, ensure};
use std::{
    net::{SocketAddr, TcpListener},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Default)]
pub struct ProcessControl(Arc<Inner>);
#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl ProcessControl {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    pub fn spawn(&self, command: &mut Command) -> Result<u32> {
        let mut child = self.0.child.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(!self.is_cancelled(), "OCR process was cancelled");
        ensure!(
            child.is_none(),
            "An OCR child is already owned by this control"
        );
        crate::platform::hide_console(command);
        let owned = command
            .spawn()
            .context("Could not start the local OCR process")?;
        let pid = owned.id();
        *child = Some(owned);
        Ok(pid)
    }
    pub fn try_wait(&self) -> Result<Option<ExitStatus>> {
        self.0
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .context("The OCR process is no longer owned")?
            .try_wait()
            .context("Could not inspect the OCR process")
    }
    pub fn owned_pid(&self) -> Result<u32> {
        ensure!(!self.is_cancelled(), "OCR process was cancelled");
        let mut child = self.0.child.lock().unwrap_or_else(|e| e.into_inner());
        let child = child
            .as_mut()
            .context("The OCR process is no longer owned")?;
        ensure!(child.try_wait()?.is_none(), "The local OCR process exited");
        Ok(child.id())
    }
    pub fn ensure_listener(&self, port: u16) -> Result<()> {
        let root = self.owned_pid()?;
        let listener = listener_owner(port)?.context("The local OCR listener is not ready")?;
        ensure!(
            listener == root || descendant_of(listener, root)?,
            "The OCR port belongs to a different process"
        );
        ensure!(
            self.owned_pid()? == root,
            "The OCR process changed during connection verification"
        );
        Ok(())
    }
    pub fn stop(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        if let Some(mut child) = self
            .0
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            reap(&mut child);
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(mut child) = self
            .child
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            reap(&mut child);
        }
    }
}
fn reap(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(windows)]
    {
        let mut kill = Command::new("taskkill.exe");
        kill.args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = crate::platform::hide_console(&mut kill).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub fn ensure_port_unused(port: u16) -> Result<()> {
    let probe = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .context("The local OCR port is already in use")?;
    drop(probe);
    Ok(())
}

#[cfg(windows)]
fn descendant_of(pid: u32, root: u32) -> Result<bool> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;
    struct Snapshot(windows::Win32::Foundation::HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    let _guard = Snapshot(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut parents = std::collections::BTreeMap::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    Ok(descendant_in(&parents, pid, root))
}
#[cfg(any(windows, test))]
fn descendant_in(parents: &std::collections::BTreeMap<u32, u32>, mut pid: u32, root: u32) -> bool {
    for _ in 0..64 {
        if pid == root {
            return true;
        }
        let Some(&parent) = parents.get(&pid) else {
            return false;
        };
        if parent == pid || parent == 0 {
            return false;
        }
        pid = parent;
    }
    false
}
#[cfg(not(windows))]
fn descendant_of(_pid: u32, _root: u32) -> Result<bool> {
    Ok(false)
}

#[cfg(windows)]
fn listener_owner(port: u16) -> Result<Option<u32>> {
    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        #[link_name = "GetExtendedTcpTable"]
        fn tcp_table(
            table: *mut std::ffi::c_void,
            size: *mut u32,
            order: i32,
            family: u32,
            class: i32,
            reserved: u32,
        ) -> u32;
    }
    let mut size = 0;
    let _ = unsafe { tcp_table(std::ptr::null_mut(), &mut size, 0, 2, 3, 0) };
    for _ in 0..3 {
        ensure!(
            (4..=1_048_576).contains(&size),
            "Invalid OCR listener table size"
        );
        let mut table = vec![0u32; (size as usize).div_ceil(4)];
        let status = unsafe { tcp_table(table.as_mut_ptr().cast(), &mut size, 0, 2, 3, 0) };
        if status == 122 {
            continue;
        }
        ensure!(status == 0, "Could not verify the local OCR listener");
        let count = table[0] as usize;
        ensure!(count <= (table.len() - 1) / 6, "Invalid OCR listener table");
        for row in table[1..].as_chunks::<6>().0.iter().take(count) {
            if u16::from_be(row[2] as u16) == port
                && (row[1] == 0 || row[1] == u32::from_ne_bytes([127, 0, 0, 1]))
            {
                return Ok(Some(row[5]));
            }
        }
        return Ok(None);
    }
    anyhow::bail!("The local OCR listener table kept changing")
}
#[cfg(not(windows))]
fn listener_owner(_port: u16) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descendant_check_stays_inside_the_owned_tree_and_bounds_cycles() {
        let parents = [(4, 1), (5, 4), (6, 5), (7, 8), (8, 7)].into();
        assert!(descendant_in(&parents, 6, 4));
        assert!(!descendant_in(&parents, 6, 9));
        assert!(!descendant_in(&parents, 7, 4));
    }
    #[test]
    fn cancelled_control_never_spawns_a_child() {
        let control = ProcessControl::new();
        control.stop();
        assert!(
            control
                .spawn(&mut Command::new("an-executable-that-must-not-run"))
                .is_err()
        );
        assert!(control.owned_pid().is_err());
    }
}
