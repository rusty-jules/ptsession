#![cfg(target_os = "macos")]

use std::ffi::{CStr, CString};
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use CoreFoundation_sys::{
    base::kCFAllocatorDefault,
    base::{CFStringRef, CFTypeRef},
    dictionary::CFDictionarySetValue,
    dictionary::CFMutableDictionaryRef,
    string::{kCFStringEncodingUTF8, CFStringCreateWithCString},
    CFDictionaryRef, CFRelease, CFStringGetCStringPtr,
};
use IOKit_sys::{
    kIOMasterPortDefault, IOObjectConformsTo, IORegistryEntryCreateCFProperty,
    IORegistryEntryGetParentEntry, IOServiceGetMatchingService, IOServiceMatching,
};

/// Set up Vector metrics shipping

#[repr(transparent)]
struct Statfs(libc::statfs);

#[derive(Debug)]
pub struct VolumeInfo {
    pub mount_name: String,
    pub device_serial: Option<String>,
}

fn statfs_for(path: &Path) -> io::Result<Statfs> {
    let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut st = MaybeUninit::<libc::statfs>::zeroed();
    let res = unsafe { libc::statfs(cpath.as_ptr(), st.as_mut_ptr()) };
    if res != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Statfs(unsafe { st.assume_init() }))
}

/// Retrieve volume identifiers
fn get_mount_and_device(path: &Path) -> io::Result<(String, String)> {
    let Statfs(st) = statfs_for(path)?;
    // safety: those are guaranteed null‑terminated C strings
    let mnt = unsafe { CStr::from_ptr(st.f_mntonname.as_ptr()) }
        .to_str()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "invalid statsf utf‑8"))?
        .to_owned();
    let dev = unsafe { CStr::from_ptr(st.f_mntfromname.as_ptr()) }
        .to_str()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "invalid statsf utf‑8"))?
        .to_owned();
    Ok((mnt, dev))
}

fn serial_for_volume(dev: &str) -> io::Result<String> {
    let disk = Path::new(dev)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .trim_end_matches(char::is_numeric)
        .trim_end_matches(char::is_alphabetic);

    // 1) Build the matching dictionary as a CFMutableDictionaryRef
    let iomedia = CString::new("IOMedia").unwrap();
    let matching = unsafe { IOServiceMatching(iomedia.as_ptr()) as CFMutableDictionaryRef };

    // 2) Make your CFString keys/vals with the *same* sys‐crate CFStringRef:
    let bsd_name = CString::new("BSD Name").unwrap();
    let cf_key: CFStringRef = unsafe {
        CFStringCreateWithCString(
            kCFAllocatorDefault,
            bsd_name.as_ptr(),
            kCFStringEncodingUTF8,
        )
    };
    let disk_str = CString::new(disk).unwrap();
    let cf_val: CFStringRef = unsafe {
        CFStringCreateWithCString(
            kCFAllocatorDefault,
            disk_str.as_ptr(),
            kCFStringEncodingUTF8,
        )
    };

    // 3) Now insert into that dictionary with the sys‐crate function:
    unsafe {
        CFDictionarySetValue(matching, cf_key as CFTypeRef, cf_val as CFTypeRef);
    }

    // 4) Find the IOMedia, climb to IOBlockStorageDevice...
    let mut service =
        unsafe { IOServiceGetMatchingService(kIOMasterPortDefault, matching as CFDictionaryRef) };
    if service == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no media"));
    }

    let ioblockstoragedevice = CString::new("IOBlockStorageDevice").unwrap();
    let ioservice = CString::new("IOService").unwrap();
    while unsafe { IOObjectConformsTo(service, ioblockstoragedevice.as_ptr()) } == 0 {
        let mut parent = 0;
        unsafe {
            IORegistryEntryGetParentEntry(service, ioservice.as_ptr(), &mut parent);
        }
        service = parent;
        if service == 0 {
            return Err(io::Error::new(io::ErrorKind::Other, "no block device"));
        }
    }

    let serial_string = CString::new("Serial Number").unwrap();
    let serial_key: CFStringRef = unsafe {
        CFStringCreateWithCString(
            kCFAllocatorDefault,
            serial_string.as_ptr(),
            kCFStringEncodingUTF8,
        )
    };
    let cf_serial =
        unsafe { IORegistryEntryCreateCFProperty(service, serial_key, std::ptr::null_mut(), 0) };
    if cf_serial.is_null() {
        return Err(io::Error::new(io::ErrorKind::Other, "no serial property"));
    }
    let serial = unsafe {
        // CFRelease takes a CFTypeRef
        let s_ref = cf_serial as CFTypeRef;
        // Convert that back to a Rust String via core_foundation (or CString)
        let cstr =
            CString::from_raw(CFStringGetCStringPtr(s_ref as _, /* UTF8 */ 0x08000100) as *mut _);
        CFRelease(cf_serial);
        cstr.into_string().unwrap_or_default()
    };
    Ok(serial)
}

pub fn get_volume_info(path: &Path) -> io::Result<VolumeInfo> {
    let (mount_name, device) = get_mount_and_device(path)?;

    Ok(VolumeInfo {
        mount_name,
        device_serial: serial_for_volume(&device).ok(),
    })
}
