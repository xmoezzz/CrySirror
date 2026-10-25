//! Embedded host metadata, without launching helpers or opening ambient files.
//! Calendar/local timezone and CPU record queries retain their original host
//! metadata semantics in the trusted VM. This module only replaces physical
//! memory queries previously implemented with sysctl/vm_stat or /proc reads.
use std::io;

#[cfg(target_os = "macos")]
pub fn physical_memory_bytes() -> io::Result<(u64, u64)> {
    let mut total = 0u64;
    let mut total_size = std::mem::size_of::<u64>();
    // SAFETY: the fixed read-only sysctl key returns one u64 into a correctly
    // sized buffer; no mutable sysctl value or caller-controlled key is used.
    let total_result = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut total as *mut u64).cast(),
            &mut total_size,
            std::ptr::null_mut(),
            0,
        )
    };
    if total_result != 0 {
        return Err(io::Error::last_os_error());
    }
    if total_size != std::mem::size_of::<u64>() || total == 0 {
        return Err(io::Error::other("invalid physical memory metadata"));
    }
    let mut statistics = std::mem::MaybeUninit::<libc::vm_statistics64>::zeroed();
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: the Darwin libc bindings define the exact vm_statistics64
    // layout/count. mach_host_self returns a send right retained below until
    // the synchronous statistics query completes.
    let host = unsafe { libc::mach_host_self() };
    let result = unsafe {
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            statistics.as_mut_ptr().cast(),
            &mut count,
        )
    };
    unsafe extern "C" {
        // Verified against the macOS SDK mach/mach_port.h declaration; only
        // this small signature is used, no SDK files are vendored.
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
    }
    // SAFETY: releases only the send right returned above, never a VM handle.
    unsafe {
        mach_port_deallocate(libc::mach_task_self(), host);
    }
    if result != libc::KERN_SUCCESS || count < libc::HOST_VM_INFO64_COUNT {
        return Err(io::Error::other("kernel VM statistics unavailable"));
    }
    // SAFETY: success and the complete declared count were checked above.
    let statistics = unsafe { statistics.assume_init() };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(io::Error::last_os_error());
    }
    // Same fields summed by the original vm_stat parser, with the actual
    // kernel page size (16 KiB on this Apple Silicon host).
    let pages = u64::from(statistics.free_count)
        .saturating_add(u64::from(statistics.inactive_count))
        .saturating_add(u64::from(statistics.speculative_count))
        .saturating_add(u64::from(statistics.purgeable_count));
    Ok((total, pages.saturating_mul(page_size as u64).min(total)))
}

#[cfg(target_os = "linux")]
pub fn physical_memory_bytes() -> io::Result<(u64, u64)> {
    let mut info = std::mem::MaybeUninit::<libc::sysinfo>::zeroed();
    // SAFETY: sysinfo receives its declared writable structure. No path is
    // opened; the kernel supplies total/free/buffered memory counters.
    if unsafe { libc::sysinfo(info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    let unit = u64::from(info.mem_unit);
    let total = (info.totalram as u64).saturating_mul(unit);
    let available = (info.freeram as u64)
        .saturating_add(info.bufferram as u64)
        .saturating_mul(unit)
        .min(total);
    if total == 0 {
        return Err(io::Error::other("invalid physical memory metadata"));
    }
    Ok((total, available))
}

#[cfg(target_os = "windows")]
pub fn physical_memory_bytes() -> io::Result<(u64, u64)> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    let mut value = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    // SAFETY: unchanged native Win32 metadata query with its declared layout.
    if unsafe { GlobalMemoryStatusEx(&mut value) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((value.total_phys, value.avail_phys.min(value.total_phys)))
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn physical_memory_bytes() -> io::Result<(u64, u64)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "physical memory host metadata unavailable",
    ))
}
