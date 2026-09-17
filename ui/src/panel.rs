use objc::{class, msg_send, sel, sel_impl};

/*
 * What the guest ought to pace its frames to: the panel the window is on, in millihertz,
 * which is the unit QEMU's UI info carries.  The panel's own maximum is the number the
 * guest wants, not the rate its current mode happens to be running at - a ProMotion
 * display drops to 60 Hz whenever the content is still, and a guest told that would cap
 * itself there for as long as the desktop sat idle.
 */
#[allow(unexpected_cfgs)]
pub(crate) fn refresh_rate() -> u32 {
    let screen: *mut objc::runtime::Object = unsafe { msg_send![class!(NSScreen), mainScreen] };

    if screen.is_null() {
        return 0;
    }
    let frames: usize = unsafe { msg_send![screen, maximumFramesPerSecond] };

    (frames * 1000).min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(unexpected_cfgs)]
    fn the_panel_is_a_refresh_rate_the_guest_can_use() {
        let rate = refresh_rate();

        assert!(
            (24_000..=1_000_000).contains(&rate),
            "the panel's rate came back as {rate} millihertz"
        );
    }
}
