use objc::{class, msg_send, sel, sel_impl};

#[allow(unexpected_cfgs)]
pub(crate) fn held() -> bool {
    let buttons: usize = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };

    buttons != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(unexpected_cfgs)]
    fn nothing_is_held_when_no_button_is_pressed() {
        assert_eq!(class!(NSEvent).name(), "NSEvent", "AppKit is not loaded");
        assert!(!held());
    }
}
