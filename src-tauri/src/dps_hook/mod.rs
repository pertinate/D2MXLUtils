//! Inline hook on `D2Common::STATLIST_SetUnitStat` (Ord10887). Captures
//! every monster HP write in both SP and MP.
//!
//! See `docs/dps-meter-reverse-engineering.md` for the call chain
//! and `docs/superpowers/specs/2026-05-07-dps-meter-design.md` for the
//! design rationale.

#![cfg(target_os = "windows")]

mod ring;
mod trampoline;

use std::ffi::c_void;
use std::sync::Mutex;

use windows::core::s;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Diagnostics::Debug::{
    FlushInstructionCache, ReadProcessMemory, WriteProcessMemory,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
    PAGE_EXECUTE_READWRITE,
};

use crate::logger::{error as log_error, info as log_info};
use crate::offsets::{d2client, d2common};

/// 1024 × 16 B = 16 KB — far more than a single scanner tick can
/// accumulate from one client.
const RING_CAPACITY: u32 = 1024;
/// 32 KB: trampoline + helper (~300 B) + ring (~16 KB), with headroom
/// for a future RING_CAPACITY bump.
const REGION_SIZE: usize = 0x8000;

#[derive(Debug, Clone, Copy)]
pub struct HookEvent {
    /// `GetTickCount` snapshot at hook time. Native u32 — wraps every
    /// ~49 days, which `DpsMeter::snapshot` handles via `wrapping_sub`.
    pub ts_ms: u32,
    pub unit_id: u32,
    pub delta_raw: u32,
    /// Template `wMaxHP[difficulty]` from MonStats.txt for this monster's
    /// class. Used together with `monster_level` to estimate runtime
    /// damage (server-side actual max HP isn't available client-side in MP).
    pub max_hp: u16,
    /// Runtime monster level (`stat 12`) read from the unit's stat list
    /// at hook time. Zero if the trampoline didn't find the stat — caller
    /// then falls back to ×1 scaling (legacy behaviour).
    pub monster_level: u16,
}

pub struct DpsHook {
    state: Mutex<Option<HookState>>,
}

#[allow(dead_code)]
struct HookState {
    /// Borrowed handle — closed by `D2Context::Drop`, not by us.
    process: HANDLE,
    d2common_base: usize,
    region: usize,
    region_size: usize,
    ring_addr: usize,
    ring_capacity: u32,
    saved_bytes: [u8; 5],
    read_tail: u32,
}

// SAFETY: `HANDLE` is a kernel-object reference. Its Win32 R/W ops are
// thread-safe; we never close it. Mirrors `process::ProcessHandle`.
unsafe impl Send for HookState {}

impl DpsHook {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }

    pub fn is_installed(&self) -> bool {
        self.state.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Install the inline hook at `D2Common.dll + STATLIST_SET_UNIT_STAT`.
    /// Allocates a region in the D2 process for trampoline + ring buffer,
    /// then atomically patches `Ord10887`'s first 5 bytes with
    /// `E9 rel32 → trampoline`.
    pub fn install(
        &self,
        process: HANDLE,
        d2common_base: usize,
        d2client_base: usize,
    ) -> Result<(), String> {
        let mut state_lock = self
            .state
            .lock()
            .map_err(|e| format!("hook state mutex poisoned: {}", e))?;
        if state_lock.is_some() {
            return Err("DPS hook already installed".into());
        }

        // KERNEL32 is mapped at the same VA in every process on Windows
        // (incl. WoW64), so reading its export from our own module table
        // gives the right address for the remote process.
        let kernel32 = unsafe { GetModuleHandleA(s!("kernel32.dll")) }
            .map_err(|e| format!("GetModuleHandleA(kernel32): {}", e))?;
        let get_tick_count = unsafe { GetProcAddress(kernel32, s!("GetTickCount")) }
            .ok_or_else(|| "GetProcAddress(GetTickCount) returned null".to_string())?;
        let get_tick_count_addr = get_tick_count as usize;

        let ord10887_addr = d2common_base + d2common::STATLIST_SET_UNIT_STAT;
        let ord10887_resume = ord10887_addr + 5;
        let difficulty_addr = d2client_base + d2client::DIFFICULTY;

        let region_ptr = unsafe {
            VirtualAllocEx(
                process,
                None,
                REGION_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        };
        if region_ptr.is_null() {
            return Err("VirtualAllocEx for DPS hook region failed".into());
        }
        let region = region_ptr as usize;

        let blob = trampoline::build(&trampoline::BuildParams {
            ord10887_resume,
            blob_base: region,
            ring_capacity: RING_CAPACITY,
            difficulty_addr,
            get_tick_count_addr,
        });
        if blob.bytes.len() > REGION_SIZE {
            let _ = unsafe { VirtualFreeEx(process, region_ptr, 0, MEM_RELEASE) };
            return Err(format!(
                "trampoline blob size {} > REGION_SIZE {}",
                blob.bytes.len(),
                REGION_SIZE
            ));
        }

        if let Err(e) = write_remote(process, region, &blob.bytes) {
            let _ = unsafe { VirtualFreeEx(process, region_ptr, 0, MEM_RELEASE) };
            return Err(format!("write trampoline blob: {}", e));
        }
        unsafe {
            let _ = FlushInstructionCache(process, Some(region as *const c_void), blob.bytes.len());
        }

        // Save original prologue BEFORE patching, so uninstall can restore it.
        let mut saved = [0u8; 5];
        if let Err(e) = read_remote(process, ord10887_addr, &mut saved) {
            let _ = unsafe { VirtualFreeEx(process, region_ptr, 0, MEM_RELEASE) };
            return Err(format!("read original Ord10887 prologue: {}", e));
        }
        // If MXL has rebuilt D2Common, the prologue won't match and our
        // saved bytes wouldn't make sense to restore. Bail loudly rather
        // than risk bricking the game.
        const EXPECTED_PROLOGUE: [u8; 5] = [0x8B, 0x44, 0x24, 0x0C, 0x53];
        if saved != EXPECTED_PROLOGUE {
            let _ = unsafe { VirtualFreeEx(process, region_ptr, 0, MEM_RELEASE) };
            return Err(format!(
                "Ord10887 prologue mismatch: expected {:02X?}, got {:02X?} \
                 — D2Common.dll may have been updated; RVA needs reverification",
                EXPECTED_PROLOGUE, saved
            ));
        }

        // 5 bytes is well under a cache line, so the patch write is
        // single-instruction-safe even with concurrent execution.
        let trampoline_abs = region + blob.trampoline_offset;
        let patch_eip_after = ord10887_addr + 5;
        let rel: i32 = (trampoline_abs as i64 - patch_eip_after as i64) as i32;
        let mut patch = [0u8; 5];
        patch[0] = 0xE9;
        patch[1..5].copy_from_slice(&rel.to_le_bytes());

        if let Err(e) = write_remote(process, ord10887_addr, &patch) {
            let _ = unsafe { VirtualFreeEx(process, region_ptr, 0, MEM_RELEASE) };
            return Err(format!("write E9 patch at Ord10887: {}", e));
        }
        unsafe {
            let _ = FlushInstructionCache(process, Some(ord10887_addr as *const c_void), 5);
        }

        log_info(&format!(
            "DPS hook installed: trampoline @ 0x{:08X}, ring @ 0x{:08X}, target @ 0x{:08X}",
            trampoline_abs,
            region + blob.ring_offset,
            ord10887_addr
        ));

        *state_lock = Some(HookState {
            process,
            d2common_base,
            region,
            region_size: REGION_SIZE,
            ring_addr: region + blob.ring_offset,
            ring_capacity: RING_CAPACITY,
            saved_bytes: saved,
            read_tail: 0,
        });

        Ok(())
    }

    /// Restore Ord10887's original prologue and free the trampoline region.
    /// Idempotent: returns Ok if not currently installed.
    pub fn uninstall(&self) -> Result<(), String> {
        let mut state_lock = self
            .state
            .lock()
            .map_err(|e| format!("hook state mutex poisoned: {}", e))?;
        let state = match state_lock.take() {
            Some(s) => s,
            None => return Ok(()),
        };

        let ord10887_addr = state.d2common_base + d2common::STATLIST_SET_UNIT_STAT;

        if let Err(e) = write_remote(state.process, ord10887_addr, &state.saved_bytes) {
            log_error(&format!(
                "DPS hook uninstall: failed to restore Ord10887 prologue: {} \
                 — leaking region 0x{:08X} to avoid use-after-free",
                e, state.region
            ));
            // Don't free the region — the trampoline may still be reachable.
            return Err(format!("restore Ord10887: {}", e));
        }
        unsafe {
            let _ = FlushInstructionCache(state.process, Some(ord10887_addr as *const c_void), 5);
        }

        // Drain any thread mid-trampoline before freeing the region.
        // 50 ms >> trampoline runtime (~200 cycles).
        std::thread::sleep(std::time::Duration::from_millis(50));

        let region_ptr = state.region as *mut c_void;
        unsafe {
            if let Err(e) = VirtualFreeEx(state.process, region_ptr, 0, MEM_RELEASE) {
                log_error(&format!("DPS hook uninstall: VirtualFreeEx failed: {}", e));
            }
        }

        log_info("DPS hook uninstalled");
        Ok(())
    }

    /// Drain pending events from the in-process ring buffer.
    pub fn drain(&self) -> Vec<HookEvent> {
        let mut state_lock = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let state = match state_lock.as_mut() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let mut reader = ring::RingReader {
            process: state.process,
            ring_addr: state.ring_addr,
            capacity: state.ring_capacity,
            tail: state.read_tail,
        };
        let events = reader.drain();
        state.read_tail = reader.tail;
        events
    }
}

impl Default for DpsHook {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DpsHook {
    fn drop(&mut self) {
        let _ = self.uninstall();
    }
}

pub(crate) fn write_remote(handle: HANDLE, addr: usize, data: &[u8]) -> Result<(), String> {
    let mut written = 0usize;
    unsafe {
        WriteProcessMemory(
            handle,
            addr as *const c_void,
            data.as_ptr() as *const c_void,
            data.len(),
            Some(&mut written),
        )
        .map_err(|e| format!("WriteProcessMemory failed: {}", e))?;
    }
    if written != data.len() {
        return Err(format!(
            "WriteProcessMemory: wrote {} of {} bytes",
            written,
            data.len()
        ));
    }
    Ok(())
}

pub(crate) fn read_remote(handle: HANDLE, addr: usize, buf: &mut [u8]) -> Result<(), String> {
    let mut read = 0usize;
    unsafe {
        ReadProcessMemory(
            handle,
            addr as *const c_void,
            buf.as_mut_ptr() as *mut c_void,
            buf.len(),
            Some(&mut read),
        )
        .map_err(|e| format!("ReadProcessMemory failed: {}", e))?;
    }
    if read != buf.len() {
        return Err(format!(
            "ReadProcessMemory: read {} of {} bytes",
            read,
            buf.len()
        ));
    }
    Ok(())
}
