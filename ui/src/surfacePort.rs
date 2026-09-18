use core_foundation::base::TCFType;
use core_video::pixel_buffer::CVPixelBuffer;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

type MachPort = u32;
type KernReturn = c_int;

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferGetIOSurface(pixel_buffer: *mut c_void) -> *mut c_void;
}

#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceCreateMachPort(surface: *mut c_void) -> MachPort;
    fn IOSurfaceGetID(surface: *mut c_void) -> u32;
}

extern "C" {
    static bootstrap_port: MachPort;
    fn bootstrap_register(
        bootstrap: MachPort,
        name: *const c_char,
        service: MachPort,
    ) -> KernReturn;
    #[cfg(test)]
    fn IOSurfaceLock(surface: *mut c_void, options: u32, seed: *mut u32) -> KernReturn;
    #[cfg(test)]
    fn IOSurfaceUnlock(surface: *mut c_void, options: u32, seed: *mut u32) -> KernReturn;
}

/*
 * The surface behind a pixel buffer.  The buffer owns it, so this is a borrow rather than
 * a handle of its own: core-video's accessor wraps it under the create rule, which is a
 * reference it was never given, and the surface dies with the first release.
 */
fn surface(pixel_buffer: &CVPixelBuffer) -> Option<*mut c_void> {
    let surface = unsafe {
        CVPixelBufferGetIOSurface(pixel_buffer.as_concrete_TypeRef() as *mut c_void)
    };

    (!surface.is_null()).then_some(surface)
}

pub(crate) fn id(pixel_buffer: &CVPixelBuffer) -> u32 {
    surface(pixel_buffer)
        .map(|surface| unsafe { IOSurfaceGetID(surface) })
        .unwrap_or(0)
}

/*
 * The console locks the surface it is writing into for the length of the write, so a
 * window that draws a surface the console holds is drawing one that is busy.  The ring is
 * what keeps those apart; these are what a harness needs to put them together again.
 */
#[cfg(test)]
pub(crate) fn lock(pixel_buffer: &CVPixelBuffer) -> bool {
    surface(pixel_buffer)
        .map(|surface| unsafe { IOSurfaceLock(surface, 0, std::ptr::null_mut()) } == 0)
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn unlock(pixel_buffer: &CVPixelBuffer) {
    if let Some(surface) = surface(pixel_buffer) {
        unsafe {
            IOSurfaceUnlock(surface, 0, std::ptr::null_mut());
        }
    }
}

/*
 * An IOSurface is not a file descriptor, so there is nothing to pass over the socket: the
 * mach port is published under a bootstrap name and the console looks the name up, which
 * is why the surface is named rather than handed over.  A name outlives the task that
 * registered it, so a surface that is remade always gets a name of its own.
 */
pub(crate) fn publish(pixel_buffer: &CVPixelBuffer, name: &str) -> Result<(), String> {
    let surface = surface(pixel_buffer)
        .ok_or_else(|| "the pixel buffer has no surface".to_string())?;
    let name = CString::new(name).map_err(|error| format!("unusable surface name: {error}"))?;
    let port = unsafe { IOSurfaceCreateMachPort(surface) };

    if port == 0 {
        return Err("no mach port for the surface".to_string());
    }
    let registered = unsafe { bootstrap_register(bootstrap_port, name.as_ptr(), port) };
    if registered != 0 {
        return Err(format!("the session refused the name ({registered})"));
    }
    Ok(())
}
